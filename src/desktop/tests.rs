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
fn trash_restores_titles_and_permanent_delete_leaves_other_recovery_entries() {
    let home = tempfile::tempdir().expect("home");
    let store = home.path().join(".codex/sessions");
    std::fs::create_dir_all(&store).expect("store");
    for id in ["keep", "restore", "permanent"] {
        std::fs::write(
            store.join(format!("{id}.jsonl")),
            format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"/tmp\"}}}}\n"
            ),
        )
        .expect("fixture");
    }
    let client = DesktopClient {
        home: home.path().to_owned(),
        settings: Settings::default(),
        catalog: Mutex::new(None),
        search_generation: AtomicU64::new(0),
    };
    assert!(
        client
            .delete_session("codex:keep".to_owned(), true, "Y".to_owned())
            .is_err()
    );
    assert!(store.join("keep.jsonl").exists());
    let catalog = SessionCatalog::scan(home.path()).expect("catalog");
    catalog
        .set_title(
            resolve(&catalog, "codex:restore").expect("session"),
            "Custom display title",
        )
        .expect("title");
    let catalog = SessionCatalog::scan(home.path()).expect("updated catalog");
    for id in ["keep", "restore"] {
        catalog
            .delete_session(resolve(&catalog, &format!("codex:{id}")).expect("session"))
            .expect("trash");
    }
    catalog
        .delete_session_permanently(resolve(&catalog, "codex:permanent").expect("session"))
        .expect("permanent delete");
    assert!(!store.join("permanent.jsonl").exists());
    let entries = client.trash_sessions().expect("trash list");
    assert_eq!(entries.len(), 2);
    assert!(
        !entries
            .iter()
            .any(|entry| entry.target == "codex:permanent")
    );
    let entry = entries
        .iter()
        .find(|entry| entry.target == "codex:restore")
        .expect("recoverable");
    assert_eq!(entry.title, "Custom display title");
    client.restore_session(entry.key.clone()).expect("restore");
    assert!(store.join("restore.jsonl").exists());
    let restored = SessionCatalog::scan(home.path()).expect("restored catalog");
    assert_eq!(
        resolve(&restored, "codex:restore")
            .expect("restored")
            .title
            .as_deref(),
        Some("Custom display title")
    );
    assert_eq!(client.trash_sessions().expect("remaining").len(), 1);
}

#[test]
fn tools_system_and_error_records_cannot_create_context_forks() {
    let home = tempfile::tempdir().expect("home");
    let path = home.path().join(".codex/sessions/test.jsonl");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    std::fs::write(
        &path,
        "{\"type\":\"session_meta\",\"payload\":{\"id\":\"test\",\"cwd\":\"/tmp\"}}\n",
    )
    .expect("fixture");
    let catalog = SessionCatalog::scan(home.path()).expect("catalog");
    let mut detail = SessionDetail {
        session: resolve(&catalog, "codex:test").expect("session").clone(),
        messages: Vec::new(),
    };
    for kind in [
        SessionMessageKind::ToolCall,
        SessionMessageKind::ToolResult,
        SessionMessageKind::System,
        SessionMessageKind::Skill,
        SessionMessageKind::Error,
    ] {
        detail.messages = vec![message(kind, "must not fork")];
        assert!(!kind.can_fork());
        assert!(prepare_message_fork(&detail, 0, &AgentKind::Codex).is_err());
        assert!(catalog.fork_native(&detail, 0, None).is_err());
    }
}

#[test]
fn both_deletion_modes_fail_closed_for_an_unconfirmed_live_provider() {
    let home = tempfile::tempdir().expect("home");
    let source = home.path().join(".codex/sessions/protected.jsonl");
    std::fs::create_dir_all(source.parent().expect("parent")).expect("store");
    let record =
        "{\"type\":\"session_meta\",\"payload\":{\"id\":\"protected\",\"cwd\":\"/tmp\"}}\n";
    std::fs::write(&source, record).expect("fixture");
    let client = DesktopClient {
        home: home.path().to_owned(),
        settings: Settings::default(),
        catalog: Mutex::new(None),
        search_generation: AtomicU64::new(0),
    };
    let preview = client
        .resume_preview("codex:protected".to_owned())
        .expect("resume command");
    assert_eq!(preview.launch.arguments, ["resume", "protected"]);
    assert_eq!(preview.launch.directory, "/tmp");
    let live = crate::process::LiveAgent {
        kind: AgentKind::Codex,
        process: crate::process::ProcessSnapshot {
            pid: u32::MAX,
            parent_pid: None,
            executable: "/bin/codex".into(),
            command: vec!["codex".to_owned()],
            cwd: None,
            started_at: 0,
            run_time: 0,
            cpu_percent: 0.0,
            memory_bytes: 0,
            status: "Running".to_owned(),
        },
    };
    for permanent in [false, true] {
        assert!(
            client
                .delete_with_live_agents("codex:protected", permanent, std::slice::from_ref(&live))
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(&source).expect("still intact"),
            record
        );
        assert!(client.trash_sessions().expect("trash").is_empty());
    }
}

#[test]
fn restore_rejects_manifest_destination_and_stored_path_escapes_before_moving_files() {
    let home = tempfile::tempdir().expect("home");
    let outside = tempfile::tempdir().expect("outside");
    let source = home.path().join(".codex/sessions/day/safe.jsonl");
    std::fs::create_dir_all(source.parent().expect("parent")).expect("store");
    std::fs::write(
        &source,
        "{\"type\":\"session_meta\",\"payload\":{\"id\":\"safe\",\"cwd\":\"/tmp\"}}\n",
    )
    .expect("fixture");
    let catalog = SessionCatalog::scan(home.path()).expect("catalog");
    catalog
        .delete_session(resolve(&catalog, "codex:safe").expect("session"))
        .expect("trash");
    let entry = crate::session::list_trashed_sessions(home.path())
        .expect("entries")
        .remove(0);
    let manifest = home
        .path()
        .join(".config/mena/trash/sessions")
        .join(&entry.key)
        .join("session.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).expect("manifest")).expect("decode");
    value["moved"][0]["from"] = serde_json::json!(outside.path().join("escape.jsonl"));
    std::fs::write(&manifest, serde_json::to_vec(&value).expect("encode")).expect("tamper");
    assert!(crate::session::restore_session(home.path(), &entry.key).is_err());
    assert!(!outside.path().join("escape.jsonl").exists());
    value["moved"][0]["from"] = serde_json::json!(source);
    value["moved"][0]["stored"] = serde_json::json!("../../escape");
    std::fs::write(&manifest, serde_json::to_vec(&value).expect("encode")).expect("tamper");
    assert!(crate::session::restore_session(home.path(), &entry.key).is_err());
    assert!(!source.exists());
    assert!(manifest.exists());
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
