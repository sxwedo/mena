pub(crate) mod app;
pub(crate) mod event;
pub(crate) mod render;

use anyhow::{Context, Result};
use crossterm::event::{Event, KeyCode, KeyModifiers};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use crate::session::{
    AgentSession, DeletionSummary, DetailScope, SessionDetail, SessionProtection,
};
use crate::settings::SessionDetailColorSettings;
use crate::tui::common::{ManagedTerminal, SessionDetailTheme, is_key_press};

pub(crate) use self::app::*;
pub(crate) use self::event::*;
pub(crate) use self::render::*;

pub(crate) fn manage_sessions(
    sessions: Vec<AgentSession>,
    protection: SessionProtection,
    detail_colors: &SessionDetailColorSettings,
    mut load_detail: impl FnMut(&AgentSession) -> Result<SessionDetail>,
    mut export: impl FnMut(&SessionDetail, DetailScope) -> Result<PathBuf>,
    mut copy: impl FnMut(&SessionDetail, DetailScope) -> Result<()>,
    mut delete: impl FnMut(&AgentSession) -> Result<DeletionSummary>,
) -> Result<Option<AgentSession>> {
    run_session_browser(
        sessions,
        protection.exact_active_targets,
        protection.protected_targets,
        SessionDetailTheme::from(detail_colors),
        SessionBrowserCallbacks {
            load_detail: Some(&mut load_detail),
            export: Some(&mut export),
            copy: Some(&mut copy),
            delete: Some(&mut delete),
        },
    )
}

fn run_session_browser(
    sessions: Vec<AgentSession>,
    active_targets: BTreeSet<String>,
    protected_targets: BTreeSet<String>,
    detail_theme: SessionDetailTheme,
    mut callbacks: SessionBrowserCallbacks<'_>,
) -> Result<Option<AgentSession>> {
    let mut app = SessionsApp::new_with_detail_theme(
        sessions,
        active_targets,
        protected_targets,
        detail_theme,
    );
    let mut terminal = ManagedTerminal::enter_with_native_selection()?;
    let mut tick: usize = 0;
    loop {
        terminal
            .terminal
            .draw(|frame| draw_sessions(frame, &mut app, tick))
            .context("failed to draw session browser")?;

        tick = tick.wrapping_add(1);

        // While a transcript search runs incrementally, drive it one batch per
        // frame instead of blocking on event::read. Esc cancels; any other key
        // is swallowed until the scan finishes. Each tick redraws the spinner.
        if pump_search(&mut app, callbacks.load_detail.as_mut())
            .context("failed to advance transcript search")?
        {
            continue;
        }

        if !crossterm::event::poll(Duration::from_millis(40))
            .context("failed to poll terminal input")?
        {
            continue;
        }

        let input = crossterm::event::read().context("failed to read terminal input")?;
        if app.mode == BrowserMode::Detail {
            for input in read_detail_event_batch(input)? {
                let action = handle_detail_browser_event(
                    &mut app,
                    &input,
                    &mut callbacks.export,
                    &mut callbacks.copy,
                );
                if action == DetailAction::Resume {
                    return Ok(app.selected_session().cloned());
                }
                if app.mode != BrowserMode::Detail {
                    break;
                }
            }
            continue;
        }
        match input {
            Event::Key(key) if is_key_press(&key) => {
                if app.mode == BrowserMode::Search {
                    if key.code == KeyCode::Enter {
                        // Enter commits the query. If a transcript loader is
                        // available, kick off an incremental full-text search
                        // (animated, Esc-cancellable); otherwise fall back to a
                        // synchronous scalar-only filter.
                        start_message_search(&mut app, callbacks.load_detail.is_some());
                        app.mode = BrowserMode::Browse;
                    } else {
                        handle_search_key(&mut app, key);
                    }
                    continue;
                }
                if app.mode == BrowserMode::ConfirmDelete {
                    handle_confirm_delete_key(&mut app, key, &mut callbacks.delete);
                    continue;
                }
                app.status = None;
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(None),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(None);
                    }
                    KeyCode::Up | KeyCode::Char('k') => app.previous(),
                    KeyCode::Down | KeyCode::Char('j') => app.next(),
                    KeyCode::PageUp => app.move_by(-10),
                    KeyCode::PageDown => app.move_by(10),
                    KeyCode::Home => app.first(),
                    KeyCode::End => app.last(),
                    KeyCode::Char('/') => app.mode = BrowserMode::Search,
                    KeyCode::Char('g') => app.cycle_grouping(),
                    KeyCode::Char('r') => {
                        return Ok(app.selected_session().cloned());
                    }
                    KeyCode::Enter | KeyCode::Char('i') => {
                        if let Some(DisplayRow::GroupHeader { project, .. }) = app.selected_row() {
                            app.toggle_project_collapse(&project);
                        } else if let Some(session) = app.selected_session().cloned()
                            && let Some(load_detail) = callbacks.load_detail.as_deref_mut()
                        {
                            match load_detail(&session) {
                                Ok(detail) => app.open_detail(detail),
                                Err(error) => {
                                    app.status = Some(StatusMessage::error(format!(
                                        "Failed to load session details: {error:#}"
                                    )));
                                }
                            }
                        }
                    }
                    KeyCode::Char('d') => {
                        app.request_delete();
                    }
                    _ => {}
                }
            }
            Event::Paste(value) if app.mode == BrowserMode::Search => app.append_search(&value),
            Event::Mouse(mouse) if app.mode == BrowserMode::Browse => {
                handle_session_mouse(&mut app, mouse.kind);
            }
            _ => {}
        }
    }
}
