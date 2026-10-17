
use std::collections::BTreeMap;
use std::path::PathBuf;

use super::McpApp;
use crate::mcp::{
    McpDetail, McpProbe, McpProbeStatus, McpRegistration, McpSourceFormat, McpTimeouts,
    McpToolPolicy, McpTransport,
};

fn registration(name: &str, provider: &str, description: &str) -> McpRegistration {
    McpRegistration {
        selector: format!("{provider}:user:{name}"),
        name: name.to_owned(),
        provider: provider.to_owned(),
        scope: "user".to_owned(),
        source: PathBuf::from(format!("/{provider}/config.json")),
        source_format: McpSourceFormat::Json,
        transport: McpTransport::Stdio,
        enabled: true,
        valid: true,
        display_name: None,
        description: Some(description.to_owned()),
        command: Some(format!("{name}-server")),
        args: Vec::new(),
        url: None,
        cwd: None,
        timeouts: McpTimeouts::default(),
        authentication: Vec::new(),
        environment: Vec::new(),
        headers: Vec::new(),
        tool_policy: McpToolPolicy::default(),
        options: BTreeMap::new(),
        extra_fields: Vec::new(),
        warnings: Vec::new(),
    }
}

#[test]
fn search_selects_the_matching_registration_for_the_detail_pane() {
    let mut app = McpApp::new(vec![
        registration("docs", "codex", "documentation"),
        registration("codegraph", "claude", "repository graph"),
    ]);

    app.query = "graph".to_owned();
    app.recompute_filter();

    assert_eq!(app.visible, [1]);
    assert_eq!(
        app.selected_registration().map(|entry| entry.name.as_str()),
        Some("codegraph")
    );
}

#[test]
fn completed_probe_stays_bound_to_its_registration_after_selection_moves() {
    let first = registration("first", "codex", "first server");
    let second = registration("second", "claude", "second server");
    let mut app = McpApp::new(vec![first.clone(), second]);
    let index = app.begin_probe().expect("probe request");
    app.select_next();

    app.finish_probe(
        index,
        Ok(McpDetail {
            registration: first,
            probe: Some(McpProbe {
                status: McpProbeStatus::Success,
                duration_ms: 7,
                protocol_version: Some("2025-11-25".to_owned()),
                server: None,
                capabilities: None,
                instructions: None,
                tools: Vec::new(),
                prompts: Vec::new(),
                resources: Vec::new(),
                resource_templates: Vec::new(),
                warnings: Vec::new(),
                error: None,
            }),
        }),
    );

    assert_eq!(
        app.selected_registration().map(|entry| entry.name.as_str()),
        Some("second")
    );
    assert!(
        app.selected_detail_text()
            .expect("second detail")
            .contains("Runtime metadata: not probed")
    );

    app.select_previous();
    assert!(
        app.selected_detail_text()
            .expect("first detail")
            .contains("Runtime metadata: success (7ms)")
    );
}

#[test]
fn completed_source_action_refreshes_the_catalog() {
    let original = registration("docs", "codex", "old description");
    let mut refreshed = original.clone();
    refreshed.command = Some("new-server".to_owned());
    let mut app = McpApp::new(vec![original]);

    app.finish_source_action(0, "edit", "edited", Ok(vec![refreshed]));

    assert_eq!(
        app.selected_registration()
            .and_then(|entry| entry.command.as_deref()),
        Some("new-server")
    );
    assert!(
        app.notice
            .as_ref()
            .is_some_and(|notice| !notice.error && notice.message.starts_with("edited "))
    );
}

#[test]
fn completed_delete_rebuilds_indices_and_keeps_a_valid_selection() {
    let first = registration("first", "codex", "first");
    let second = registration("second", "codex", "second");
    let mut app = McpApp::new(vec![first.clone(), second]);
    app.select_next();
    app.begin_delete();

    app.finish_delete(1, Ok(vec![first]));

    assert_eq!(app.registrations.len(), 1);
    assert_eq!(app.selected_catalog_index(), Some(0));
    assert_eq!(app.pending_delete, None);
    assert!(
        app.notice
            .as_ref()
            .is_some_and(|notice| !notice.error && notice.message == "deleted codex:user:second")
    );
}

#[test]
fn source_edit_and_delete_keep_comment_and_owner_boundaries_distinct() {
    let mut jsonc = registration("docs", "opencode", "docs");
    jsonc.source_format = McpSourceFormat::Jsonc;
    let mut app = McpApp::new(vec![jsonc]);

    assert_eq!(app.begin_external_edit(), Some(0));
    app.begin_delete();
    assert_eq!(app.pending_delete, None);
    assert!(app.notice.as_ref().is_some_and(|notice| notice.error));

    let mut plugin = registration("docs", "claude", "docs");
    plugin.scope = "plugin".to_owned();
    let mut app = McpApp::new(vec![plugin]);

    assert_eq!(app.begin_external_edit(), None);
    assert!(app.notice.as_ref().is_some_and(|notice| notice.error));
}

#[test]
fn detail_scroll_is_not_limited_by_the_terminal_u16_offset() {
    let mut app = McpApp::new(vec![registration("large", "codex", "large catalog")]);
    let beyond_u16 = usize::from(u16::MAX) + 25;
    app.detail_max_scroll = beyond_u16;

    app.scroll_detail(isize::MAX);

    assert_eq!(app.detail_scroll, beyond_u16);
}
