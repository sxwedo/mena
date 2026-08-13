use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use crate::AgentKind;
use anyhow::Result;
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};
use ratatui::widgets::Cell;

use super::common::*;
use super::mcp::{app::McpApp, render::draw_mcp};
use super::session::*;
use crate::mcp::{McpRegistration, McpSourceFormat, McpTimeouts, McpToolPolicy, McpTransport};
use crate::session::{
    AgentSession, DeletionSummary, DetailScope, ResponseMetrics, SessionDetail, SessionMessage,
    SessionMessageKind, SessionMessageMetrics, TokenUsage,
};
use crate::settings::{ConfigColor, SessionDetailColorSettings};
use crate::tui::skill::app::{SkillRow, SkillsApp};

#[test]
fn mcp_browser_renders_a_searchable_list_and_static_detail() {
    let mut app = McpApp::new(vec![fixture_mcp_registration("codegraph")]);
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).expect("test terminal");

    terminal
        .draw(|frame| draw_mcp(frame, &mut app))
        .expect("draw MCP browser");

    let screen = buffer_text(terminal.backend().buffer(), 100, 24);
    assert!(screen.contains("MENA MCP"));
    assert!(screen.contains("codegraph"));
    assert!(screen.contains("Runtime metadata: not probed"));
    assert!(screen.contains("p Probe"));
}

#[test]
fn mcp_browser_renders_the_basic_configuration_editor() {
    let mut app = McpApp::new(vec![fixture_mcp_registration("codegraph")]);
    app.begin_edit();
    let mut terminal = Terminal::new(TestBackend::new(110, 28)).expect("test terminal");

    terminal
        .draw(|frame| draw_mcp(frame, &mut app))
        .expect("draw MCP editor");

    let screen = buffer_text(terminal.backend().buffer(), 110, 28);
    assert!(screen.contains("Edit MCP basics: codex:user:codegraph"));
    assert!(screen.contains("/Users/test/.codex/config.toml"));
    assert!(screen.contains("Enabled"));
    assert!(screen.contains("Command"));
    assert!(screen.contains("Arguments (JSON array)"));
    assert!(screen.contains("Ctrl+S Save"));
}

#[test]
fn session_layout_displays_titles_and_filters_by_them() {
    let session = fixture_session();
    let mut app = SessionsApp::new(vec![session], BTreeSet::default());
    app.query = "rendering".to_owned();
    app.recompute_filter();
    assert_eq!(app.filtered.len(), 1);

    let mut terminal = Terminal::new(TestBackend::new(100, 18)).expect("test terminal");
    terminal
        .draw(|frame| draw_sessions(frame, &mut app, 0))
        .expect("draw sessions");
    let screen = buffer_text(terminal.backend().buffer(), 100, 18);
    assert!(screen.contains("Fix terminal rendering"));
    assert!(screen.contains("d delete"));
    assert!(screen.lines().all(|line| line.chars().count() == 100));
}

#[test]
fn session_target_is_first_and_visible_at_eighty_columns() {
    let mut session = fixture_session();
    session.id = "019fbd66-e95f-7dd2-b9b4-37a27a61c272".to_owned();
    let target = session.target();
    let mut app = SessionsApp::new(vec![session], BTreeSet::default());
    let mut terminal = Terminal::new(TestBackend::new(80, 18)).expect("test terminal");

    terminal
        .draw(|frame| draw_sessions(frame, &mut app, 0))
        .expect("draw sessions");

    let screen = buffer_text(terminal.backend().buffer(), 80, 18);
    assert!(screen.contains(&target));
    assert_eq!(
        session_columns(80).first().map(|column| column.label),
        Some("TARGET")
    );
}

#[test]
fn detail_navigation_scrolls_without_changing_the_selected_session() {
    let first = fixture_session();
    let mut second = first.clone();
    second.id = "second-session".to_owned();
    let mut app = SessionsApp::new(vec![first.clone(), second], BTreeSet::default());
    app.open_detail(SessionDetail {
        session: first,
        messages: vec![SessionMessage {
            kind: SessionMessageKind::User,
            timestamp: None,
            model: None,
            metrics: SessionMessageMetrics::default(),
            content: "line\n".repeat(40),
        }],
    });
    app.detail_max_scroll = 20;
    let selected = app.table_state.selected();

    handle_detail_key(
        &mut app,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        None,
        None,
    );

    assert_eq!(app.table_state.selected(), selected);
    assert_eq!(app.detail_scroll, 1);
    assert_eq!(app.mode, BrowserMode::Detail);
}

#[test]
fn detail_resume_requests_the_same_selected_session_as_the_outer_list() {
    let first = fixture_session();
    let mut second = first.clone();
    second.id = "second-session".to_owned();
    let mut app = SessionsApp::new(vec![first.clone(), second], BTreeSet::default());
    app.open_detail(SessionDetail {
        session: first.clone(),
        messages: Vec::new(),
    });

    let action = handle_detail_event(
        &mut app,
        &Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)),
        None,
        None,
    );

    assert_eq!(action, DetailAction::Resume);
    assert_eq!(app.selected_session(), Some(&first));
}

#[test]
fn detail_mode_renders_complete_metadata_and_chat_in_a_popup() {
    let mut session = fixture_session();
    session.started_at = Some("2026-08-01T01:02:03Z".to_owned());
    session.cost_usd = Some(1.25);
    let mut app = SessionsApp::new(vec![session.clone()], BTreeSet::default());
    app.open_detail(SessionDetail {
        session,
        messages: vec![
            SessionMessage {
                kind: SessionMessageKind::User,
                timestamp: Some("2026-08-01T01:02:04Z".to_owned()),
                model: None,
                metrics: SessionMessageMetrics::default(),
                content: "complete first question".to_owned(),
            },
            SessionMessage {
                kind: SessionMessageKind::Assistant,
                timestamp: Some("2026-08-01T01:02:05Z".to_owned()),
                model: Some("gpt-5.5".to_owned()),
                metrics: SessionMessageMetrics::default(),
                content: "complete first answer".to_owned(),
            },
            SessionMessage {
                kind: SessionMessageKind::Assistant,
                timestamp: Some("2026-08-01T01:02:06Z".to_owned()),
                model: Some("gpt-5.6".to_owned()),
                metrics: SessionMessageMetrics {
                    response: Some(ResponseMetrics {
                        duration_ms: Some(125_450),
                        time_to_first_token_ms: Some(400),
                        cost_usd: Some(0.42),
                        finish_reason: Some("stop".to_owned()),
                        retry_count: Some(2),
                        tokens: TokenUsage {
                            total: Some(123_456),
                            input: Some(100_000),
                            output: Some(23_456),
                            cache_read: Some(80_000),
                            cache_write: Some(500),
                            cache_write_5m: Some(400),
                            cache_write_1h: Some(100),
                            reasoning: Some(456),
                            tool: Some(100),
                        },
                        ..ResponseMetrics::default()
                    }),
                    ..SessionMessageMetrics::default()
                },
                content: "complete second answer".to_owned(),
            },
        ],
    });
    let mut terminal = Terminal::new(TestBackend::new(100, 50)).expect("test terminal");
    terminal
        .draw(|frame| draw_sessions(frame, &mut app, 0))
        .expect("draw details");

    let screen = buffer_text(terminal.backend().buffer(), 100, 50);
    for expected in [
        "Session details",
        "Started",
        "2026-08-01T01:02:03Z",
        "Tokens",
        "125500000",
        "Cost",
        "$1.2500",
        "Conversation — 3 shown (conversation only)",
        "Model usage (1 models)",
        "gpt-5.6 · 1 responses · duration 2m 05.5s · avg TTFT 400ms",
        "ASSISTANT · gpt-5.5",
        "ASSISTANT · gpt-5.6 · 2m 05.5s · 123,456 tokens · $0.4200",
        "input 100,000 · output 23,456 · cache read 80,000 · cache write 500 (5m 400 · 1h 100)",
        "Response: status completed · stop reason stop · TTFT 400ms · retries 2",
        "complete first question",
        "complete first answer",
        "complete second answer",
        "Shift+↑/↓ msg",
        "p chat",
        "Shift+P all",
        "c copy",
        "r resume",
        "e export",
        "Esc close",
    ] {
        assert!(screen.contains(expected), "missing {expected:?}\n{screen}");
    }
    for redundant_hint in ["↑/↓ scroll", "PgUp/PgDn page", "Home/End jump"] {
        assert!(
            !screen.contains(redundant_hint),
            "redundant detail hint remained: {redundant_hint:?}\n{screen}"
        );
    }
}

#[test]
fn detail_preview_defaults_to_conversation_only_and_hides_tool_messages() {
    let session = fixture_session();
    let messages = vec![
        SessionMessage {
            kind: SessionMessageKind::User,
            timestamp: None,
            model: None,
            metrics: SessionMessageMetrics::default(),
            content: "visible user question".to_owned(),
        },
        SessionMessage {
            kind: SessionMessageKind::ToolCall,
            timestamp: None,
            model: None,
            metrics: SessionMessageMetrics::default(),
            content: "hidden tool call body".to_owned(),
        },
        SessionMessage {
            kind: SessionMessageKind::ToolResult,
            timestamp: None,
            model: None,
            metrics: SessionMessageMetrics::default(),
            content: "hidden tool result body".to_owned(),
        },
        SessionMessage {
            kind: SessionMessageKind::Assistant,
            timestamp: None,
            model: Some("gpt-5.5".to_owned()),
            metrics: SessionMessageMetrics::default(),
            content: "visible assistant answer".to_owned(),
        },
    ];
    let mut app = SessionsApp::new(vec![session.clone()], BTreeSet::default());
    app.open_detail(SessionDetail { session, messages });
    assert_eq!(app.preview_scope, DetailScope::Conversation);

    let mut terminal = Terminal::new(TestBackend::new(80, 30)).expect("test terminal");
    terminal
        .draw(|frame| draw_sessions(frame, &mut app, 0))
        .expect("draw conversation-only details");

    let screen = buffer_text(terminal.backend().buffer(), 80, 30);
    assert!(screen.contains("visible user question"));
    assert!(screen.contains("visible assistant answer"));
    assert!(screen.contains("2 tool/system hidden"));
    assert!(
        !screen.contains("hidden tool call body"),
        "tool calls must be hidden in the default preview\n{screen}"
    );
    assert!(
        !screen.contains("hidden tool result body"),
        "tool results must be hidden in the default preview\n{screen}"
    );
}

#[test]
fn shift_p_reveals_all_messages_and_p_returns_to_conversation_only() {
    let session = fixture_session();
    let messages = vec![
        SessionMessage {
            kind: SessionMessageKind::User,
            timestamp: None,
            model: None,
            metrics: SessionMessageMetrics::default(),
            content: "conv user content".to_owned(),
        },
        SessionMessage {
            kind: SessionMessageKind::ToolCall,
            timestamp: None,
            model: None,
            metrics: SessionMessageMetrics::default(),
            content: "full-only tool content".to_owned(),
        },
    ];
    let mut app = SessionsApp::new(vec![session.clone()], BTreeSet::default());
    app.open_detail(SessionDetail { session, messages });

    // Shift+P (uppercase P) switches to the complete preview.
    handle_detail_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('P'), KeyModifiers::SHIFT),
        None,
        None,
    );
    assert_eq!(app.preview_scope, DetailScope::All);
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).expect("test terminal");
    terminal
        .draw(|frame| draw_sessions(frame, &mut app, 0))
        .expect("draw complete details");
    let complete = buffer_text(terminal.backend().buffer(), 80, 30);
    assert!(
        complete.contains("full-only tool content"),
        "Shift+P must reveal tool messages\n{complete}"
    );
    assert!(complete.contains("(complete)"));

    // p returns to the conversation-only preview.
    handle_detail_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
        None,
        None,
    );
    assert_eq!(app.preview_scope, DetailScope::Conversation);
    terminal
        .draw(|frame| draw_sessions(frame, &mut app, 0))
        .expect("draw conversation details again");
    let conv = buffer_text(terminal.backend().buffer(), 80, 30);
    assert!(
        !conv.contains("full-only tool content"),
        "p must re-hide tool messages\n{conv}"
    );
}

#[test]
fn detail_messages_color_headers_and_bodies_by_primary_or_supporting_kind() {
    let session = fixture_session();
    let kinds = [
        (SessionMessageKind::User, Color::LightGreen),
        (SessionMessageKind::Assistant, Color::Cyan),
        (SessionMessageKind::Skill, Color::LightYellow),
        (SessionMessageKind::ToolCall, Color::DarkGray),
        (SessionMessageKind::ToolResult, Color::DarkGray),
        (SessionMessageKind::System, Color::DarkGray),
        (SessionMessageKind::Error, Color::DarkGray),
    ];
    let messages = kinds
        .iter()
        .enumerate()
        .map(|(index, (kind, _))| SessionMessage {
            kind: *kind,
            timestamp: None,
            model: None,
            metrics: SessionMessageMetrics::default(),
            content: format!("plain-body-{index}"),
        })
        .collect();
    let mut app = SessionsApp::new(vec![session.clone()], BTreeSet::default());
    app.open_detail(SessionDetail { session, messages });
    app.preview_scope = DetailScope::All;
    let mut terminal = Terminal::new(TestBackend::new(100, 42)).expect("test terminal");

    terminal
        .draw(|frame| draw_sessions(frame, &mut app, 0))
        .expect("draw details");

    let buffer = terminal.backend().buffer();
    for (index, (kind, expected_color)) in kinds.iter().enumerate() {
        let header_position = find_text(buffer, 100, 42, kind.label()).expect("message header");
        let header = buffer.cell(header_position).expect("header cell");
        assert_eq!(header.fg, *expected_color, "{} header color", kind.label());
        assert!(
            header.modifier.contains(Modifier::BOLD),
            "{} header should be bold",
            kind.label()
        );

        let body = format!("plain-body-{index}");
        let body_position = find_text(buffer, 100, 42, &body).expect("message body");
        let body_cell = buffer.cell(body_position).expect("body cell");
        assert_eq!(body_cell.fg, *expected_color, "{body} foreground");
        assert!(!body_cell.modifier.contains(Modifier::BOLD), "{body} bold");
    }
}

#[test]
fn detail_theme_can_customize_every_text_surface_independently() {
    let session = fixture_session();
    let colors = SessionDetailColorSettings {
        border: ConfigColor::Red,
        popup_title: ConfigColor::Blue,
        metadata_key: ConfigColor::Magenta,
        metadata_value: ConfigColor::Yellow,
        conversation_header: ConfigColor::LightBlue,
        status_success: ConfigColor::LightCyan,
        footer_key: ConfigColor::White,
        footer_text: ConfigColor::Gray,
        user_header: ConfigColor::Rgb(1, 2, 3),
        user_content: ConfigColor::Indexed(123),
        ..SessionDetailColorSettings::default()
    };
    let mut app = SessionsApp::new_with_detail_theme(
        vec![session.clone()],
        BTreeSet::default(),
        BTreeSet::default(),
        SessionDetailTheme::from(&colors),
    );
    app.open_detail(SessionDetail {
        session,
        messages: vec![SessionMessage {
            kind: SessionMessageKind::User,
            timestamp: None,
            model: None,
            metrics: SessionMessageMetrics::default(),
            content: "custom user content".to_owned(),
        }],
    });
    app.detail_status = Some(StatusMessage::success("custom status".to_owned()));
    let mut terminal = Terminal::new(TestBackend::new(100, 35)).expect("test terminal");

    terminal
        .draw(|frame| draw_sessions(frame, &mut app, 0))
        .expect("draw custom details");

    let buffer = terminal.backend().buffer();
    for (text, expected) in [
        ("Session details", Color::Blue),
        ("Target", Color::Magenta),
        ("codex:session-id", Color::Yellow),
        (
            "Conversation — 1 shown (conversation only)",
            Color::LightBlue,
        ),
        ("USER", Color::Rgb(1, 2, 3)),
        ("custom user content", Color::Indexed(123)),
        ("custom status", Color::LightCyan),
        ("Shift+↑/↓", Color::White),
        ("copy", Color::Gray),
    ] {
        let position = find_text(buffer, 100, 35, text).expect("configured text");
        assert_eq!(buffer.cell(position).expect("configured cell").fg, expected);
    }
    assert_eq!(buffer.cell((2, 1)).expect("popup border").fg, Color::Red);
}

#[test]
fn detail_metadata_keys_are_pink_purple() {
    let session = fixture_session();
    let mut app = SessionsApp::new(vec![session.clone()], BTreeSet::default());
    app.open_detail(SessionDetail {
        session,
        messages: Vec::new(),
    });
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).expect("test terminal");

    terminal
        .draw(|frame| draw_sessions(frame, &mut app, 0))
        .expect("draw details");

    let buffer = terminal.backend().buffer();
    for key in ["Target", "Agent", "Title", "Project"] {
        let position = find_text(buffer, 100, 30, key).expect("metadata key");
        assert_eq!(
            buffer.cell(position).expect("metadata cell").fg,
            Color::LightMagenta,
            "{key} color"
        );
    }
}

#[test]
fn detail_mode_can_scroll_to_the_last_chat_message() {
    let session = fixture_session();
    let messages = (0..40)
        .map(|index| SessionMessage {
            kind: if index % 2 == 0 {
                SessionMessageKind::User
            } else {
                SessionMessageKind::Assistant
            },
            timestamp: None,
            model: None,
            metrics: SessionMessageMetrics::default(),
            content: format!("complete message number {index}"),
        })
        .collect();
    let mut app = SessionsApp::new(vec![session.clone()], BTreeSet::default());
    app.open_detail(SessionDetail { session, messages });
    let selected = app.table_state.selected();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test terminal");
    terminal
        .draw(|frame| draw_sessions(frame, &mut app, 0))
        .expect("draw details");

    handle_detail_key(
        &mut app,
        KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
        None,
        None,
    );
    terminal
        .draw(|frame| draw_sessions(frame, &mut app, 0))
        .expect("draw scrolled details");

    let screen = buffer_text(terminal.backend().buffer(), 80, 24);
    assert!(screen.contains("complete message number 39"));
    assert_eq!(app.table_state.selected(), selected);
    assert!(app.detail_scroll > 0);
}

#[test]
fn detail_end_reaches_content_after_word_wrapped_lines() {
    let session = fixture_session();
    let wrapped = "aaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbb cccccccccccccccc\n".repeat(20);
    let messages = vec![SessionMessage {
        kind: SessionMessageKind::Assistant,
        timestamp: None,
        model: Some("gpt-5.6".to_owned()),
        metrics: SessionMessageMetrics::default(),
        content: format!("{wrapped}FINAL DETAIL CONTENT"),
    }];
    let mut app = SessionsApp::new(vec![session.clone()], BTreeSet::default());
    app.open_detail(SessionDetail { session, messages });
    let mut terminal = Terminal::new(TestBackend::new(36, 18)).expect("test terminal");
    terminal
        .draw(|frame| draw_sessions(frame, &mut app, 0))
        .expect("draw details");

    handle_detail_key(
        &mut app,
        KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
        None,
        None,
    );
    terminal
        .draw(|frame| draw_sessions(frame, &mut app, 0))
        .expect("draw end of details");

    let screen = buffer_text(terminal.backend().buffer(), 36, 18);
    assert!(
        screen.contains("FINAL DETAIL CONTENT"),
        "End must expose the actual final detail content\n{screen}"
    );
}

#[test]
fn detail_reflows_and_keeps_the_end_reachable_after_terminal_resize() {
    let session = fixture_session();
    let wrapped = "aaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbb cccccccccccccccc\n".repeat(20);
    let messages = vec![SessionMessage {
        kind: SessionMessageKind::Assistant,
        timestamp: None,
        model: Some("gpt-5.6".to_owned()),
        metrics: SessionMessageMetrics::default(),
        content: format!("{wrapped}FINAL CONTENT AFTER RESIZE"),
    }];
    let mut app = SessionsApp::new(vec![session.clone()], BTreeSet::default());
    app.open_detail(SessionDetail { session, messages });
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).expect("test terminal");
    terminal
        .draw(|frame| draw_sessions(frame, &mut app, 0))
        .expect("draw wide details");
    let wide_max_scroll = app.detail_max_scroll;

    terminal.backend_mut().resize(36, 18);
    terminal
        .draw(|frame| draw_sessions(frame, &mut app, 0))
        .expect("draw narrow details");
    assert!(app.detail_max_scroll > wide_max_scroll);
    handle_detail_key(
        &mut app,
        KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
        None,
        None,
    );
    terminal
        .draw(|frame| draw_sessions(frame, &mut app, 0))
        .expect("draw resized detail end");

    let screen = buffer_text(terminal.backend().buffer(), 36, 18);
    assert!(screen.contains("FINAL CONTENT AFTER RESIZE"));
}

#[test]
fn detail_scrolling_reaches_content_beyond_u16_scroll_range() {
    let session = fixture_session();
    let mut content = "complete detail line\n".repeat(66_000);
    content.push_str("ABSOLUTE FINAL DETAIL LINE");
    let messages = vec![SessionMessage {
        kind: SessionMessageKind::Assistant,
        timestamp: None,
        model: Some("gpt-5.6".to_owned()),
        metrics: SessionMessageMetrics::default(),
        content,
    }];
    let mut app = SessionsApp::new(vec![session.clone()], BTreeSet::default());
    app.open_detail(SessionDetail { session, messages });
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test terminal");
    terminal
        .draw(|frame| draw_sessions(frame, &mut app, 0))
        .expect("draw details");

    for _ in 0..7_000 {
        handle_detail_key(
            &mut app,
            KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
            None,
            None,
        );
    }
    assert_eq!(app.detail_scroll, app.detail_max_scroll);
    assert!(app.detail_scroll > usize::from(u16::MAX));
    terminal
        .draw(|frame| draw_sessions(frame, &mut app, 0))
        .expect("draw end of details");

    let screen = buffer_text(terminal.backend().buffer(), 80, 24);
    assert!(
        screen.contains("ABSOLUTE FINAL DETAIL LINE"),
        "details beyond the u16 range must remain reachable"
    );
}

#[test]
fn shift_arrows_jump_between_user_and_assistant_messages_skipping_tools() {
    let session = fixture_session();
    let messages = vec![
        SessionMessage {
            kind: SessionMessageKind::User,
            timestamp: None,
            model: None,
            metrics: SessionMessageMetrics::default(),
            content: "first user message".to_owned(),
        },
        SessionMessage {
            kind: SessionMessageKind::ToolCall,
            timestamp: None,
            model: None,
            metrics: SessionMessageMetrics::default(),
            content: "tool detail\n".repeat(15),
        },
        SessionMessage {
            kind: SessionMessageKind::ToolResult,
            timestamp: None,
            model: None,
            metrics: SessionMessageMetrics::default(),
            content: "tool result\n".repeat(15),
        },
        SessionMessage {
            kind: SessionMessageKind::Assistant,
            timestamp: None,
            model: Some("gpt-5.5".to_owned()),
            metrics: SessionMessageMetrics::default(),
            content: "assistant answer".to_owned(),
        },
    ];
    let mut app = SessionsApp::new(vec![session.clone()], BTreeSet::default());
    app.open_detail(SessionDetail { session, messages });
    app.preview_scope = DetailScope::All;
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test terminal");
    terminal
        .draw(|frame| draw_sessions(frame, &mut app, 0))
        .expect("draw details");

    handle_detail_key(
        &mut app,
        KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT),
        None,
        None,
    );
    let first_user_scroll = app.detail_scroll;
    assert!(first_user_scroll > 1, "should jump past metadata");

    handle_detail_key(
        &mut app,
        KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT),
        None,
        None,
    );
    let assistant_scroll = app.detail_scroll;
    assert!(
        assistant_scroll > first_user_scroll + 10,
        "should skip tool messages"
    );

    handle_detail_key(
        &mut app,
        KeyEvent::new(KeyCode::Up, KeyModifiers::SHIFT),
        None,
        None,
    );
    assert_eq!(app.detail_scroll, first_user_scroll);
}

#[test]
fn detail_scroll_bursts_are_coalesced_and_key_repeats_do_not_keep_scrolling() {
    let session = fixture_session();
    let mut app = SessionsApp::new(vec![session.clone()], BTreeSet::default());
    app.open_detail(SessionDetail {
        session,
        messages: Vec::new(),
    });
    app.detail_max_scroll = 200;
    let mut events = vec![Event::Key(KeyEvent::new_with_kind(
        KeyCode::Down,
        KeyModifiers::NONE,
        KeyEventKind::Press,
    ))];
    events.extend((0..100).map(|_| {
        Event::Key(KeyEvent::new_with_kind(
            KeyCode::Down,
            KeyModifiers::NONE,
            KeyEventKind::Repeat,
        ))
    }));
    events.extend((0..100).map(|_| {
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 10,
            row: 10,
            modifiers: KeyModifiers::NONE,
        })
    }));

    for event in coalesce_detail_events(events) {
        handle_detail_event(&mut app, &event, None, None);
    }

    assert_eq!(
        app.detail_scroll, 4,
        "one key press and one three-line wheel step should be applied"
    );
}

#[test]
fn alternate_scroll_arrow_bursts_are_coalesced() {
    let events = (0..64).map(|_| Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));

    let coalesced = coalesce_detail_events(events);

    assert_eq!(coalesced.len(), 1);
    assert!(matches!(
        coalesced.first(),
        Some(Event::Key(KeyEvent {
            code: KeyCode::Down,
            kind: KeyEventKind::Press,
            ..
        }))
    ));
}

#[test]
fn exporting_from_detail_keeps_selection_scroll_and_popup_open() {
    let first = fixture_session();
    let mut second = first.clone();
    second.id = "second-session".to_owned();
    let mut app = SessionsApp::new(vec![first.clone(), second], BTreeSet::default());
    app.open_detail(SessionDetail {
        session: first,
        messages: Vec::new(),
    });
    app.detail_max_scroll = 20;
    app.detail_scroll = 7;
    let selected = app.table_state.selected();
    let mut export =
        |_detail: &SessionDetail, _scope: DetailScope| Ok(PathBuf::from("/tmp/session-export.md"));

    handle_detail_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
        Some(&mut export),
        None,
    );

    assert_eq!(app.mode, BrowserMode::Detail);
    assert_eq!(app.table_state.selected(), selected);
    assert_eq!(app.detail_scroll, 7);
    assert!(app.detail.is_some());
    assert!(app.detail_status.as_ref().is_some_and(|status| {
        status.text == "Exported conversation only: /tmp/session-export.md"
            && status.style.fg == Some(Color::Green)
    }));
}

#[test]
fn copying_from_detail_copies_the_complete_session_and_keeps_context() {
    let session = fixture_session();
    let mut app = SessionsApp::new(vec![session.clone()], BTreeSet::default());
    app.open_detail(SessionDetail {
        session,
        messages: vec![SessionMessage {
            kind: SessionMessageKind::Assistant,
            timestamp: None,
            model: Some("gpt-5.6".to_owned()),
            metrics: SessionMessageMetrics::default(),
            content: "complete clipboard tail".to_owned(),
        }],
    });
    app.detail_max_scroll = 20;
    app.detail_scroll = 7;
    let selected = app.table_state.selected();
    let mut copied_tail = None;
    let mut copy = |detail: &SessionDetail, _scope: DetailScope| {
        copied_tail = detail
            .messages
            .last()
            .map(|message| message.content.clone());
        Ok(())
    };

    handle_detail_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
        None,
        Some(&mut copy),
    );

    assert_eq!(copied_tail.as_deref(), Some("complete clipboard tail"));
    assert_eq!(app.mode, BrowserMode::Detail);
    assert_eq!(app.table_state.selected(), selected);
    assert_eq!(app.detail_scroll, 7);
    assert!(app.detail.is_some());
    assert!(app.detail_status.as_ref().is_some_and(|status| {
        status.text == "Copied conversation only to clipboard"
            && status.style.fg == Some(Color::Green)
    }));
}

#[test]
fn command_c_is_left_to_the_terminal_native_selection() {
    let session = fixture_session();
    let mut app = SessionsApp::new(vec![session.clone()], BTreeSet::default());
    app.open_detail(SessionDetail {
        session,
        messages: Vec::new(),
    });
    let mut copy_called = false;
    let mut copy = |_detail: &SessionDetail, _scope: DetailScope| {
        copy_called = true;
        Ok(())
    };

    handle_detail_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::SUPER),
        None,
        Some(&mut copy),
    );

    assert!(!copy_called, "Command+C must remain a terminal shortcut");
    assert!(app.detail_status.is_none());
}

#[test]
fn failed_detail_copy_keeps_context_and_reports_a_red_error() {
    let session = fixture_session();
    let mut app = SessionsApp::new(vec![session.clone()], BTreeSet::default());
    app.open_detail(SessionDetail {
        session,
        messages: Vec::new(),
    });
    app.detail_max_scroll = 20;
    app.detail_scroll = 7;
    let selected = app.table_state.selected();
    let mut copy = |_detail: &SessionDetail, _scope: DetailScope| -> Result<()> {
        anyhow::bail!("clipboard unavailable")
    };

    handle_detail_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
        None,
        Some(&mut copy),
    );

    assert_eq!(app.mode, BrowserMode::Detail);
    assert_eq!(app.table_state.selected(), selected);
    assert_eq!(app.detail_scroll, 7);
    assert!(app.detail.is_some());
    assert!(app.detail_status.as_ref().is_some_and(|status| {
        status.text.contains("Copy failed: clipboard unavailable")
            && status.style.fg == Some(Color::Red)
    }));
}

#[test]
fn failed_detail_export_keeps_context_and_reports_a_red_error() {
    let session = fixture_session();
    let mut app = SessionsApp::new(vec![session.clone()], BTreeSet::default());
    app.open_detail(SessionDetail {
        session,
        messages: Vec::new(),
    });
    app.detail_max_scroll = 20;
    app.detail_scroll = 7;
    let selected = app.table_state.selected();
    let mut export = |_detail: &SessionDetail, _scope: DetailScope| -> Result<PathBuf> {
        anyhow::bail!("permission denied")
    };

    handle_detail_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE),
        Some(&mut export),
        None,
    );

    assert_eq!(app.mode, BrowserMode::Detail);
    assert_eq!(app.table_state.selected(), selected);
    assert_eq!(app.detail_scroll, 7);
    assert!(app.detail.is_some());
    assert!(app.detail_status.as_ref().is_some_and(|status| {
        status.text.contains("Export failed: permission denied")
            && status.style.fg == Some(Color::Red)
    }));
}

#[test]
fn running_sessions_cannot_enter_delete_confirmation() {
    let session = fixture_session();
    let target = session.target();
    let mut app = SessionsApp::new(vec![session], BTreeSet::from([target]));

    app.request_delete();

    assert_eq!(app.mode, BrowserMode::Browse);
    assert!(
        app.status
            .as_ref()
            .is_some_and(|status| status.text.contains("running agent"))
    );
}

#[test]
fn confirmed_deletion_removes_all_duplicate_catalog_rows() {
    let session = fixture_session();
    let mut duplicate = session.clone();
    duplicate.path = PathBuf::from("/tmp/duplicate-session.jsonl");
    let mut app = SessionsApp::new(vec![session.clone(), duplicate], BTreeSet::new());

    app.deleted(
        &session,
        DeletionSummary {
            files: 2,
            directories: 1,
            index_records: 3,
        },
    );

    assert!(app.sessions.is_empty());
    assert!(app.filtered.is_empty());
    assert!(app.status.as_ref().is_some_and(|status| {
        status
            .text
            .contains("2 files, 1 directories, 3 index records")
    }));
}

fn fixture_session() -> AgentSession {
    AgentSession {
        kind: AgentKind::Codex,
        id: "session-id".to_owned(),
        title: Some("Fix terminal rendering".to_owned()),
        project: Some(PathBuf::from("/work/project")),
        path: PathBuf::from("/tmp/session.jsonl"),
        started_at: None,
        updated_at: 1,
        tokens: Some(125_500_000),
        cost_usd: None,
    }
}

/// Build a minimal session with the given id/title and a distinct transcript path,
/// so `load_detail` can return different message bodies per session.
fn transcript_session(id: &str, title: &str, path: &str) -> AgentSession {
    AgentSession {
        kind: AgentKind::Codex,
        id: id.to_owned(),
        title: Some(title.to_owned()),
        project: Some(PathBuf::from("/work/project")),
        path: PathBuf::from(path),
        started_at: None,
        updated_at: 1,
        tokens: None,
        cost_usd: None,
    }
}

#[test]
fn message_search_includes_sessions_whose_transcripts_match() {
    // Neither session's scalar fields mention "rewrite"; only the first
    // session's transcript does. Enter-style commit must surface it.
    let sessions = vec![
        transcript_session("a", "Alpha work", "/tmp/a.jsonl"),
        transcript_session("b", "Beta work", "/tmp/b.jsonl"),
    ];
    let mut app = SessionsApp::new(sessions, BTreeSet::default());
    app.query = "rewrite".to_owned();
    app.recompute_filter();
    // Scalar-only: no hits yet.
    assert!(app.filtered.is_empty());

    let mut load_detail = |session: &AgentSession| {
        let content = if session.id == "a" {
            "we should rewrite the parser"
        } else {
            "leave the parser as-is"
        };
        Ok(SessionDetail {
            session: session.clone(),
            messages: vec![SessionMessage {
                kind: SessionMessageKind::User,
                timestamp: None,
                model: None,
                metrics: SessionMessageMetrics::default(),
                content: content.to_owned(),
            }],
        })
    };
    assert!(start_message_search(&mut app, true));
    while !step_message_search(&mut app, &mut load_detail) {}

    assert_eq!(app.filtered.len(), 1);
    assert_eq!(app.filtered[0], 0);
    assert_eq!(app.message_search.as_ref().map(BTreeSet::len), Some(1));
    assert!(app.search_in_progress.is_none());
    assert!(app.status.as_ref().is_some_and(|status| !status.is_error));
}

#[test]
fn editing_the_query_clears_committed_message_search() {
    let sessions = vec![transcript_session("a", "Alpha", "/tmp/a.jsonl")];
    let mut app = SessionsApp::new(sessions, BTreeSet::default());
    app.query = "rewrite".to_owned();
    let mut load_detail = |session: &AgentSession| {
        Ok(SessionDetail {
            session: session.clone(),
            messages: vec![SessionMessage {
                kind: SessionMessageKind::User,
                timestamp: None,
                model: None,
                metrics: SessionMessageMetrics::default(),
                content: "rewrite the parser".to_owned(),
            }],
        })
    };
    assert!(start_message_search(&mut app, true));
    while !step_message_search(&mut app, &mut load_detail) {}
    assert_eq!(app.filtered.len(), 1);

    // Typing one more char should drop the stale transcript search and fall
    // back to scalar-only filtering (which no longer matches).
    app.query.push('!');
    app.clear_message_search();
    assert!(app.message_search.is_none());
    assert!(app.filtered.is_empty());
}

#[test]
fn message_search_without_load_detail_is_a_noop() {
    let sessions = vec![transcript_session("a", "Alpha", "/tmp/a.jsonl")];
    let mut app = SessionsApp::new(sessions, BTreeSet::default());
    app.query = "rewrite".to_owned();
    // No loader: start_message_search returns false synchronously and never
    // spawns an incremental search.
    assert!(!start_message_search(&mut app, false));
    assert!(app.message_search.is_none());
    assert!(app.search_in_progress.is_none());
    assert!(app.filtered.is_empty());
}

#[test]
fn search_progress_text_reports_scanned_and_hit_counts() {
    let progress = InProgressSearch {
        query: "rewrite".to_owned(),
        hits: [1usize, 4].into_iter().collect(),
        errors: 0,
        cursor: 3,
        started: std::time::Instant::now(),
    };
    let text = search_progress_text(&progress, 5);
    assert!(text.contains("3/5 scanned"));
    assert!(text.contains("2 matches"));
    assert!(text.contains("Esc cancel"));
}

#[test]
fn abort_message_search_discards_partial_results() {
    let sessions = vec![
        transcript_session("a", "Alpha", "/tmp/a.jsonl"),
        transcript_session("b", "Beta", "/tmp/b.jsonl"),
    ];
    let mut app = SessionsApp::new(sessions, BTreeSet::default());
    app.query = "rewrite".to_owned();
    assert!(start_message_search(&mut app, true));
    // Scan one session, then cancel before finishing.
    let mut load_detail = |session: &AgentSession| {
        Ok(SessionDetail {
            session: session.clone(),
            messages: vec![SessionMessage {
                kind: SessionMessageKind::User,
                timestamp: None,
                model: None,
                metrics: SessionMessageMetrics::default(),
                content: "rewrite the parser".to_owned(),
            }],
        })
    };
    step_message_search(&mut app, &mut load_detail);
    assert!(app.search_in_progress.is_some());
    abort_message_search(&mut app);
    // No committed message search, and the footer status reflects the cancel.
    assert!(app.message_search.is_none());
    assert!(app.search_in_progress.is_none());
    assert!(app.status.as_ref().is_some_and(|status| status.is_error));
}

#[test]
fn cycle_grouping_toggles_between_flat_and_project() {
    let sessions = vec![
        transcript_session("a", "Alpha", "/tmp/a.jsonl"),
        transcript_session("b", "Beta", "/tmp/b.jsonl"),
    ];
    let mut app = SessionsApp::new(sessions, BTreeSet::default());
    assert_eq!(app.grouping, Grouping::Flat);

    app.cycle_grouping();
    assert_eq!(app.grouping, Grouping::Project);
    assert!(
        app.status
            .as_ref()
            .is_some_and(|status| status.text.contains("by project"))
    );
    assert!(app.display_rows().iter().all(|row| matches!(
        row,
        DisplayRow::GroupHeader {
            collapsed: true,
            ..
        }
    )));

    app.cycle_grouping();
    assert_eq!(app.grouping, Grouping::Flat);
    assert!(
        app.status
            .as_ref()
            .is_some_and(|status| status.text.contains("flat"))
    );
}

#[test]
fn project_grouping_renders_header_rows_and_allows_selectable_group_headers() {
    // Two projects (p1, p2) with two sessions each. With Project grouping on,
    // each project renders a header row; selecting a header row makes selected_session()
    // return None.
    let mut sessions = vec![
        transcript_session("a1", "Alpha one", "/tmp/p1/a1.jsonl"),
        transcript_session("a2", "Alpha two", "/tmp/p1/a2.jsonl"),
        transcript_session("b1", "Beta one", "/tmp/p2/b1.jsonl"),
        transcript_session("b2", "Beta two", "/tmp/p2/b2.jsonl"),
    ];
    // Distinct project paths so labels differ.
    sessions[0].project = Some(PathBuf::from("/work/p1"));
    sessions[1].project = Some(PathBuf::from("/work/p1"));
    sessions[2].project = Some(PathBuf::from("/work/p2"));
    sessions[3].project = Some(PathBuf::from("/work/p2"));
    let mut app = SessionsApp::new(sessions, BTreeSet::default());
    app.grouping = Grouping::Project;
    // Select the first group header (index 0).
    app.table_state.select(Some(0));

    let mut terminal = Terminal::new(TestBackend::new(120, 20)).expect("test terminal");
    terminal
        .draw(|frame| draw_sessions(frame, &mut app, 0))
        .expect("draw sessions");
    let screen = buffer_text(terminal.backend().buffer(), 120, 20);

    // Both project headers should appear with session count.
    assert!(screen.contains("▾ /work/p1"));
    assert!(screen.contains("▾ /work/p2"));
    assert!(screen.contains("(2 sessions)"));
    // Since header (index 0) is selected, selected_session() is None.
    assert_eq!(app.selected_session(), None);
}

#[test]
fn project_grouping_allows_collapsing_and_expanding_groups() {
    let mut sessions = vec![
        transcript_session("a1", "Alpha one", "/tmp/p1/a1.jsonl"),
        transcript_session("a2", "Alpha two", "/tmp/p1/a2.jsonl"),
        transcript_session("b1", "Beta one", "/tmp/p2/b1.jsonl"),
    ];
    sessions[0].project = Some(PathBuf::from("/work/p1"));
    sessions[1].project = Some(PathBuf::from("/work/p1"));
    sessions[2].project = Some(PathBuf::from("/work/p2"));
    let mut app = SessionsApp::new(sessions, BTreeSet::default());
    app.grouping = Grouping::Project;

    // Initially: Header p1 (0), a1 (1), a2 (2), Header p2 (3), b1 (4)
    assert_eq!(app.display_rows().len(), 5);

    // Toggle collapse on /work/p1
    app.toggle_project_collapse("/work/p1");

    // Now: Header p1 (0), Header p2 (1), b1 (2)
    assert_eq!(app.display_rows().len(), 3);
    assert_eq!(
        app.display_rows()[0],
        DisplayRow::GroupHeader {
            project: "/work/p1".to_owned(),
            count: 2,
            collapsed: true
        }
    );

    let mut terminal = Terminal::new(TestBackend::new(120, 20)).expect("test terminal");
    terminal
        .draw(|frame| draw_sessions(frame, &mut app, 0))
        .expect("draw sessions");
    let screen = buffer_text(terminal.backend().buffer(), 120, 20);

    assert!(screen.contains("▸ /work/p1"));
    assert!(screen.contains("(2 sessions)"));
    assert!(!screen.contains("Alpha one"));
    // Toggle expand on /work/p1
    app.toggle_project_collapse("/work/p1");
    assert_eq!(app.display_rows().len(), 5);
}

#[test]
fn session_project_label_buckets_missing_projects() {
    let mut session = transcript_session("a", "Alpha", "/tmp/a.jsonl");
    session.project = None;
    assert_eq!(session_project_label(&session), "(no project)");
    session.project = Some(PathBuf::from("/work/x"));
    assert_eq!(session_project_label(&session), "/work/x");
}
#[test]
fn format_project_display_path_abbreviates_home_and_truncates() {
    if let Some(home) = dirs::home_dir() {
        let full_path = home.join("code/my-project").display().to_string();
        let formatted = format_project_display_path(&full_path, 40);
        assert_eq!(formatted, "~/code/my-project");

        let long_path = home
            .join("code/deeply/nested/directory/structure/my-project")
            .display()
            .to_string();
        let formatted_long = format_project_display_path(&long_path, 30);
        assert!(formatted_long.contains("..."));
        assert!(formatted_long.ends_with("my-project"));
    }
}
#[test]
fn active_session_renders_green_active_indicator() {
    let session = transcript_session("a", "Alpha", "/tmp/a.jsonl");
    let mut active_targets = BTreeSet::new();
    active_targets.insert(session.target());
    let app = SessionsApp::new(vec![session.clone()], active_targets);

    let cell = session_cell(&session, SessionColumn::Active, &app);
    let inactive_cell = session_cell(&session, SessionColumn::Agent, &app);

    // Active indicator should not be empty
    assert_ne!(cell, Cell::from(""));
    assert_eq!(
        inactive_cell,
        Cell::from(ratatui::text::Span::styled(
            "Codex",
            ratatui::style::Style::default().fg(Color::Green)
        ))
    );
}

#[test]
fn fail_closed_protection_does_not_claim_an_exact_active_session() {
    let session = transcript_session("a", "Alpha", "/tmp/a.jsonl");
    let protected_targets = BTreeSet::from([session.target()]);
    let mut app = SessionsApp::new_with_detail_theme(
        vec![session.clone()],
        BTreeSet::new(),
        protected_targets,
        SessionDetailTheme::default(),
    );

    assert_eq!(
        session_cell(&session, SessionColumn::Active, &app),
        Cell::from("")
    );
    app.request_delete();
    assert_eq!(app.mode, BrowserMode::Browse);
    assert!(app.status.as_ref().is_some_and(|status| status.is_error));
}

fn buffer_text(buffer: &ratatui::buffer::Buffer, width: u16, height: u16) -> String {
    let mut output = String::new();
    for y in 0..height {
        for x in 0..width {
            if let Some(cell) = buffer.cell((x, y)) {
                output.push_str(cell.symbol());
            }
        }
        output.push('\n');
    }
    output
}

fn fixture_mcp_registration(name: &str) -> McpRegistration {
    McpRegistration {
        selector: format!("codex:user:{name}"),
        name: name.to_owned(),
        provider: "codex".to_owned(),
        scope: "user".to_owned(),
        source: PathBuf::from("/Users/test/.codex/config.toml"),
        source_format: McpSourceFormat::Toml,
        transport: McpTransport::Stdio,
        enabled: true,
        valid: true,
        display_name: Some("CodeGraph".to_owned()),
        description: Some("Repository graph server".to_owned()),
        command: Some("codegraph-mcp".to_owned()),
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

fn find_text(
    buffer: &ratatui::buffer::Buffer,
    width: u16,
    height: u16,
    needle: &str,
) -> Option<(u16, u16)> {
    for y in 0..height {
        let row = (0..width)
            .filter_map(|x| buffer.cell((x, y)))
            .map(ratatui::buffer::Cell::symbol)
            .collect::<String>();
        if let Some(byte_index) = row.find(needle) {
            let x = row[..byte_index].chars().count();
            return u16::try_from(x).ok().map(|x| (x, y));
        }
    }
    None
}

fn fixture_skill(name: &str, dir: &std::path::Path) -> crate::skill::AgentSkill {
    crate::skill::AgentSkill {
        name: name.to_string(),
        provider: "test".to_string(),
        scope: "workspace".to_string(),
        path: dir.join("SKILL.md"),
        location: "~/.test/skills".to_string(),
        is_symlink: false,
        description: Some(format!("{name} skill description")),
        triggers: vec![name.to_string()],
        valid: true,
        children: fixture_skill_children(dir),
    }
}

fn fixture_skill_children(dir: &std::path::Path) -> Vec<crate::skill::SkillChildItem> {
    let mut children: Vec<_> = std::fs::read_dir(dir)
        .expect("read fixture skill directory")
        .map(|entry| {
            let path = entry.expect("fixture child").path();
            crate::skill::SkillChildItem {
                name: path
                    .file_name()
                    .expect("fixture child name")
                    .to_string_lossy()
                    .into_owned(),
                is_dir: path.is_dir(),
                path,
            }
        })
        .collect();
    children.sort_by(|left, right| {
        right
            .is_dir
            .cmp(&left.is_dir)
            .then_with(|| left.name.cmp(&right.name))
    });
    children
}

#[test]
fn multi_level_tree_expands_nested_directories() {
    // Build: skill_dir/{references/guide.md, references/examples/demo.md, SKILL.md}
    let tmp = std::env::temp_dir().join(format!("mena-tree-test-{}", std::process::id()));
    let skill_dir = tmp.join("myskill");
    let references = skill_dir.join("references");
    let examples = references.join("examples");
    std::fs::create_dir_all(&examples).expect("create dirs");
    std::fs::write(skill_dir.join("SKILL.md"), "# My Skill\n").expect("write skill");
    std::fs::write(references.join("guide.md"), "# Guide\n").expect("write guide");
    std::fs::write(examples.join("demo.md"), "# Demo\n").expect("write demo");

    let skill = fixture_skill("myskill", &skill_dir);
    let mut app = SkillsApp::new(vec![skill]);

    // Top-level collapsed: only the skill row.
    assert_eq!(app.visible_rows.len(), 1);
    assert!(matches!(app.visible_rows[0], SkillRow::Skill { .. }));

    // Expand the skill: references/ + SKILL.md appear (dirs first).
    app.selected_index = 0;
    app.toggle_expand();
    let rows = &app.visible_rows;
    assert_eq!(rows.len(), 3);
    assert!(matches!(
        rows[1],
        SkillRow::Item {
            is_dir: true,
            depth: 1,
            ..
        }
    ));

    // Select references dir and expand: examples/ + guide.md appear (dirs first).
    app.selected_index = 1;
    app.cache_children(references.clone(), fixture_skill_children(&references));
    app.toggle_expand();
    let rows = &app.visible_rows;
    assert!(rows.iter().any(|r| matches!(
        r,
        SkillRow::Item { depth: 2, is_dir: true, name, .. } if name == "examples"
    )));
    assert!(rows.iter().any(|r| matches!(
        r,
        SkillRow::Item { depth: 2, is_dir: false, name, .. } if name == "guide.md"
    )));

    // Expand examples: demo.md at depth 3 appears.
    let examples_idx = rows
        .iter()
        .position(|r| matches!(r, SkillRow::Item { name, .. } if name == "examples"))
        .expect("examples row");
    app.selected_index = examples_idx;
    app.cache_children(examples.clone(), fixture_skill_children(&examples));
    app.toggle_expand();
    let rows = &app.visible_rows;
    assert!(rows.iter().any(|r| matches!(
        r,
        SkillRow::Item { depth: 3, is_dir: false, name, .. } if name == "demo.md"
    )));

    // Collapse references: its subtree disappears.
    let references_idx = rows
        .iter()
        .position(|r| matches!(r, SkillRow::Item { name, .. } if name == "references"))
        .expect("references row");
    app.selected_index = references_idx;
    app.collapse_current();
    let rows = &app.visible_rows;
    assert_eq!(rows.len(), 3);
    assert!(
        !rows
            .iter()
            .any(|r| matches!(r, SkillRow::Item { name, .. } if name == "demo.md"))
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn selected_open_path_returns_containing_directory() {
    let tmp = std::env::temp_dir().join(format!("mena-open-path-test-{}", std::process::id()));
    let skill_dir = tmp.join("myskill");
    let references = skill_dir.join("references");
    std::fs::create_dir_all(&references).expect("create dirs");
    std::fs::write(skill_dir.join("SKILL.md"), "# My Skill\n").expect("write skill");
    std::fs::write(references.join("guide.md"), "# Guide\n").expect("write guide");

    let skill = fixture_skill("myskill", &skill_dir);
    let mut app = SkillsApp::new(vec![skill]);

    // 1. Top-level Skill row (SKILL.md file): opens skill_dir
    assert_eq!(app.selected_open_path(), Some(skill_dir.clone()));

    // Expand tree
    app.toggle_expand();

    // 2. Select references directory item: opens references directory
    let ref_idx = app
        .visible_rows
        .iter()
        .position(|r| matches!(r, SkillRow::Item { name, .. } if name == "references"))
        .expect("references row");
    app.selected_index = ref_idx;
    assert_eq!(app.selected_open_path(), Some(references));

    // 3. Select SKILL.md file item: opens skill_dir
    let skill_md_idx = app
        .visible_rows
        .iter()
        .position(|r| matches!(r, SkillRow::Item { name, .. } if name == "SKILL.md"))
        .expect("SKILL.md row");
    app.selected_index = skill_md_idx;
    assert_eq!(app.selected_open_path(), Some(skill_dir));

    let _ = std::fs::remove_dir_all(&tmp);
}
