use super::*;
use crate::session::{SessionMessage, SessionMessageKind, SessionMessageMetrics};
use std::collections::BTreeSet;

#[test]
fn fresh_agent_preparation_reuses_native_argv_and_validates_the_project() {
    let project = tempfile::Builder::new()
        .prefix("mena project ")
        .tempdir()
        .expect("project");
    let executable = std::env::current_exe().expect("test executable");
    let mut settings = Settings::default();
    settings.agent.custom.insert(
        "fixture".to_owned(),
        crate::settings::CustomAgentSettings {
            executables: vec![executable.display().to_string()],
            command_contains: Vec::new(),
            resume: Vec::new(),
        },
    );
    let client = DesktopClient {
        home: project.path().to_owned(),
        settings,
        catalog: Mutex::new(None),
        search_generation: AtomicU64::new(0),
    };
    let launch = client
        .prepare_agent("fixture".to_owned(), project.path().display().to_string())
        .expect("prepared")
        .launch();
    assert_eq!(launch.executable, executable.display().to_string());
    assert_eq!(launch.directory, project.path().display().to_string());
    assert!(launch.arguments.is_empty());
    assert!(launch.target.is_none());
    assert!(
        client
            .prepare_agent("unknown".to_owned(), project.path().display().to_string())
            .is_err()
    );
    assert!(
        client
            .prepare_agent("fixture".to_owned(), "../project".to_owned())
            .is_err()
    );
}

#[test]
fn desktop_export_preserves_scope_private_permissions_and_source() {
    let home = tempfile::tempdir().expect("home");
    let source = home.path().join(".codex/sessions/export.jsonl");
    std::fs::create_dir_all(source.parent().expect("parent")).expect("mkdir");
    let records = concat!(
        "{\"type\":\"session_meta\",\"payload\":{\"id\":\"export\",\"cwd\":\"/tmp\"}}\n",
        "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"USER_MARKER\"}]}}\n",
        "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"test_tool\",\"call_id\":\"test\",\"arguments\":\"TOOL_MARKER\"}}\n"
    );
    std::fs::write(&source, records).expect("fixture");
    let client = DesktopClient {
        home: home.path().to_owned(),
        settings: Settings::default(),
        catalog: Mutex::new(None),
        search_generation: AtomicU64::new(0),
    };
    let exports = tempfile::tempdir().expect("exports");
    let export = |all_messages| {
        client
            .export_session(
                "codex:export".to_owned(),
                exports.path().display().to_string(),
                all_messages,
            )
            .expect("export")
    };
    let conversation = export(false);
    let full = export(true);
    assert!(
        std::fs::read_to_string(&conversation)
            .expect("read")
            .contains("USER_MARKER")
    );
    assert!(
        !std::fs::read_to_string(&conversation)
            .expect("read")
            .contains("TOOL_MARKER")
    );
    assert!(
        std::fs::read_to_string(&full)
            .expect("read")
            .contains("TOOL_MARKER")
    );
    assert_ne!(export(false), conversation);
    assert_eq!(std::fs::read_to_string(&source).expect("source"), records);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&full)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

fn message(kind: SessionMessageKind, content: &str) -> SessionMessage {
    SessionMessage {
        kind,
        content: content.to_owned(),
        fork_point: None,
        timestamp: None,
        model: None,
        metrics: SessionMessageMetrics::default(),
    }
}

#[test]
fn fork_identity_rejects_changes_before_the_selected_message() {
    let home = tempfile::tempdir().expect("home");
    let path = home.path().join(".codex/sessions/test.jsonl");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(&path, concat!(
        "{\"type\":\"session_meta\",\"payload\":{\"id\":\"desktop-test\",\"cwd\":\"/tmp\"}}\n",
        "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"first\"}]}}\n"
    )).expect("fixture");
    let catalog = SessionCatalog::scan(home.path()).expect("catalog");
    let session = resolve(&catalog, "codex:desktop-test").expect("session");
    let mut detail = SessionDetail {
        session: session.clone(),
        messages: vec![
            message(SessionMessageKind::User, "first"),
            message(SessionMessageKind::Assistant, "selected"),
            message(SessionMessageKind::User, "later"),
        ],
    };
    let shown = DesktopSessionDetail::from_detail(&detail);
    assert_eq!(
        checked_message(&detail, &shown.revision, &shown.messages[1].id).expect("selected"),
        1
    );
    detail.messages[0].content = "changed prefix".to_owned();
    assert!(checked_message(&detail, &shown.revision, &shown.messages[1].id).is_err());
    assert!(resolve(&catalog, "codex:desktop").is_err());
}

#[test]
fn desktop_resource_reads_use_catalog_containment_and_redaction() {
    let home = tempfile::tempdir().expect("home");
    std::fs::create_dir_all(home.path().join(".codex")).expect("mkdir");
    std::fs::write(home.path().join(".codex/AGENTS.md"), "private instructions").expect("memory");
    std::fs::write(home.path().join(".codex/config.toml"),"[mcp_servers.docs]\nurl = 'https://example.com/mcp?key=DO_NOT_EXPOSE'\n[mcp_servers.docs.http_headers]\nAuthorization = 'Bearer NEVER_EXPOSE'\n").expect("mcp");
    let entries =
        resources::list(home.path(), None, DesktopResourceKind::Memories).expect("memories");
    let selected = entries
        .iter()
        .find(|entry| entry.name == "AGENTS.md")
        .expect("memory");
    let detail = resources::detail(
        home.path(),
        None,
        DesktopResourceKind::Memories,
        &selected.id,
    )
    .expect("detail");
    assert_eq!(detail.content, "private instructions");
    assert!(
        resources::detail(
            home.path(),
            None,
            DesktopResourceKind::Memories,
            "../../outside"
        )
        .is_err()
    );
    let entries = resources::list(home.path(), None, DesktopResourceKind::Mcp).expect("mcp list");
    let detail = resources::detail(home.path(), None, DesktopResourceKind::Mcp, &entries[0].id)
        .expect("static detail");
    assert!(detail.probe_status.is_none());
    assert!(!detail.content.contains("DO_NOT_EXPOSE"));
    assert!(!detail.content.contains("NEVER_EXPOSE"));
}

#[test]
fn prepared_context_keeps_only_the_prefix_and_owns_its_private_file() {
    let home = tempfile::tempdir().expect("home");
    let project = home.path().join("project");
    std::fs::create_dir_all(&project).expect("project");
    let session = AgentSession {
        kind: AgentKind::ClaudeCode,
        id: "context-test".to_owned(),
        title: Some("test".to_owned()),
        project: Some(project),
        path: home.path().join("session.jsonl"),
        related_paths: BTreeSet::default(),
        is_subagent: false,
        parent_session_id: None,
        started_at: None,
        updated_at: 0,
        tokens: Some(100),
        cost_usd: Some(1.0),
    };
    let detail = SessionDetail {
        session,
        messages: vec![
            message(SessionMessageKind::User, "INCLUDED"),
            message(SessionMessageKind::Assistant, "ALSO_INCLUDED"),
            message(SessionMessageKind::User, "EXCLUDED_LATER"),
        ],
    };
    let handoff = prepare_message_fork(&detail, 1, &AgentKind::Codex).expect("handoff");
    let path = handoff.handoff_path().expect("path").to_owned();
    let text = std::fs::read_to_string(&path).expect("context");
    assert!(text.contains("ALSO_INCLUDED"));
    assert!(!text.contains("EXCLUDED_LATER"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    drop(handoff);
    assert!(!path.exists());
}
