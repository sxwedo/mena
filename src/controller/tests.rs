use super::split_session_selector;
use std::collections::BTreeMap;

#[test]
fn grok_resume_targets_split_as_a_known_provider() {
    let custom = BTreeMap::new();
    let (provider, id) =
        split_session_selector("grok:01a05b12-60ae-7790-8716-9293782180a9", &custom);
    assert_eq!(provider, Some("grok"));
    assert_eq!(id, "01a05b12-60ae-7790-8716-9293782180a9");
}

#[test]
fn session_visibility_filters_before_limit_and_keeps_json_identity() {
    use crate::{SessionsArgs, session::SessionCatalog};
    use serde_json::json;

    let temp = tempfile::tempdir().expect("home");
    let root = temp.path().join(".codex/sessions");
    std::fs::create_dir_all(&root).expect("sessions");
    for (id, source) in [
        (
            "a-child",
            json!({"subagent": {"thread_spawn": {"parent_thread_id": "z-main"}}}),
        ),
        ("z-main", json!("cli")),
    ] {
        let meta = json!({"type": "session_meta", "payload": {"id": id, "source": source}});
        let path = root.join(format!("{id}.jsonl"));
        std::fs::write(&path, meta.to_string()).expect("session");
        std::fs::File::options()
            .write(true)
            .open(path)
            .expect("file")
            .set_modified(
                std::time::UNIX_EPOCH
                    + std::time::Duration::from_secs(if id == "a-child" { 2 } else { 1 }),
            )
            .expect("mtime");
    }
    let catalog = SessionCatalog::scan(temp.path()).expect("catalog");
    let mut args = SessionsArgs {
        limit: Some(1),
        ..SessionsArgs::default()
    };
    let visible = super::listed_sessions(&catalog, &args);
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, "z-main");
    assert_eq!(super::session_list_json(&visible[0])["is_subagent"], false);
    args.include_subagents = true;
    let visible = super::listed_sessions(&catalog, &args);
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, "a-child");
    let value = super::session_list_json(&visible[0]);
    assert_eq!(value["is_subagent"], true);
    assert_eq!(value["parent_session_id"], "z-main");
    assert_eq!(value["target"], "codex:a-child");
    assert!(crate::view::render_session_table(&visible).contains("[SubAgent]"));
}
