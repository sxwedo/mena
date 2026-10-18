use std::collections::BTreeMap;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::handle_key;
use crate::mcp::{McpRegistration, McpSourceFormat, McpTimeouts, McpToolPolicy, McpTransport};
use crate::tui::mcp::app::{McpApp, McpFocus};

fn registration(name: &str) -> McpRegistration {
    McpRegistration {
        selector: format!("codex:user:{name}"),
        name: name.to_owned(),
        provider: "codex".to_owned(),
        scope: "user".to_owned(),
        source: PathBuf::from("/codex/config.toml"),
        source_format: McpSourceFormat::Toml,
        transport: McpTransport::Stdio,
        enabled: true,
        valid: true,
        display_name: None,
        description: None,
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
fn detail_navigation_scrolls_without_moving_the_selected_registration() {
    let mut app = McpApp::new(vec![registration("first"), registration("second")]);
    app.detail_max_scroll = 20;

    handle_key(&mut app, KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    handle_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));

    assert_eq!(app.focus, McpFocus::Detail);
    assert_eq!(app.selected_catalog_index(), Some(0));
    assert_eq!(app.detail_scroll, 1);
}

#[test]
fn probe_key_emits_only_one_explicit_request_at_a_time() {
    let mut app = McpApp::new(vec![registration("first")]);

    // `p` first arms the confirmation; nothing has been started yet.
    let armed = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
    );
    assert!(matches!(armed, super::McpAction::Continue));
    assert_eq!(app.pending_probe, Some(0));
    assert_eq!(app.probe_in_progress, None);

    // A second `p` while the confirmation is open changes nothing.
    let repeat = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
    );
    assert!(matches!(repeat, super::McpAction::Continue));
    assert_eq!(app.pending_probe, Some(0));

    // Lowercase `y` confirms and emits exactly one probe request.
    let confirmed = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
    );
    assert!(matches!(confirmed, super::McpAction::Probe { index: 0 }));
    assert_eq!(app.pending_probe, None);
    assert_eq!(app.probe_in_progress, Some(0));

    // While a probe runs, `p` neither arms nor emits another request.
    let during = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
    );
    assert!(matches!(during, super::McpAction::Continue));
    assert_eq!(app.pending_probe, None);
}

#[test]
fn probe_confirmation_cancels_without_starting_anything() {
    let mut app = McpApp::new(vec![registration("first")]);

    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
    );
    let cancelled = handle_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    assert!(matches!(cancelled, super::McpAction::Continue));
    assert_eq!(app.pending_probe, None);
    assert_eq!(app.probe_in_progress, None);
}

#[test]
fn source_and_delete_keys_target_only_the_selected_registration() {
    let mut app = McpApp::new(vec![registration("first"), registration("second")]);
    app.select_next();

    let open = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE),
    );
    assert!(matches!(open, super::McpAction::Open { index: 1 }));

    let edit = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
    );
    assert!(matches!(edit, super::McpAction::Edit { index: 1 }));

    let request_delete = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE),
    );
    assert!(matches!(request_delete, super::McpAction::Continue));
    assert_eq!(app.pending_delete, Some(1));

    let uppercase = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('Y'), KeyModifiers::SHIFT),
    );
    assert!(matches!(uppercase, super::McpAction::Continue));
    assert_eq!(app.pending_delete, Some(1));

    let confirm = handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
    );
    assert!(matches!(confirm, super::McpAction::Delete { index: 1 }));
}
