pub(crate) mod app;
pub(crate) mod event;
pub(crate) mod render;

use anyhow::{Context, Result};
use crossterm::event::{Event, KeyCode, KeyModifiers};
use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::session::{
    AgentSession, DeletionSummary, DetailScope, SessionDetail, SessionProtection,
};
use crate::settings::SessionDetailColorSettings;
use crate::tui::common::{ManagedTerminal, SessionDetailTheme, is_key_press, surface_for_digit};

pub(crate) use self::app::*;
pub(crate) use self::event::*;
pub(crate) use self::render::*;

#[allow(clippy::too_many_arguments)]
pub(crate) fn manage_sessions(
    sessions: Vec<AgentSession>,
    protection: SessionProtection,
    detail_colors: &SessionDetailColorSettings,
    mut load_detail: impl FnMut(&AgentSession) -> Result<SessionDetail>,
    mut export: impl FnMut(&SessionDetail, DetailScope) -> Result<PathBuf>,
    mut copy: impl FnMut(&SessionDetail, DetailScope) -> Result<()>,
    mut delete: impl FnMut(&AgentSession) -> Result<DeletionSummary>,
    mut rename: impl FnMut(&AgentSession, &str) -> Result<Option<String>>,
) -> Result<Option<SessionBrowserResult>> {
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
            rename: Some(&mut rename),
        },
    )
}

fn run_session_browser(
    sessions: Vec<AgentSession>,
    active_targets: BTreeSet<String>,
    protected_targets: BTreeSet<String>,
    detail_theme: SessionDetailTheme,
    mut callbacks: SessionBrowserCallbacks<'_>,
) -> Result<Option<SessionBrowserResult>> {
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

        let input = crossterm::event::read().context("failed to read terminal input")?;
        if matches!(app.mode, BrowserMode::Detail | BrowserMode::DetailSearch) {
            for input in read_detail_event_batch(input)? {
                // While the detail search box is open, every key goes to the
                // query; nothing else in the popup reacts.
                if app.mode == BrowserMode::DetailSearch
                    && let Event::Key(key) = &input
                    && is_key_press(key)
                {
                    match key.code {
                        KeyCode::Enter => app.commit_detail_search(),
                        KeyCode::Esc => app.cancel_detail_search(),
                        KeyCode::Backspace => app.backspace_detail_search(),
                        KeyCode::Char(character)
                            if !key
                                .modifiers
                                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                        {
                            app.append_detail_search(character);
                        }
                        _ => {}
                    }
                    continue;
                }
                let action = handle_detail_browser_event(
                    &mut app,
                    &input,
                    &mut callbacks.export,
                    &mut callbacks.copy,
                );
                if action == DetailAction::Resume {
                    return Ok(app
                        .selected_session()
                        .cloned()
                        .map(SessionBrowserResult::Resume));
                }
                if action == DetailAction::ContinueWith {
                    return Ok(app
                        .selected_session()
                        .cloned()
                        .map(SessionBrowserResult::ContinueWith));
                }
                if action == DetailAction::Rename {
                    app.close_detail();
                    app.begin_rename();
                    break;
                }
                if app.mode != BrowserMode::Detail {
                    break;
                }
            }
            continue;
        }
        match input {
            Event::Key(key) if is_key_press(&key) => {
                // The `?` overlay explains the keymap; any key closes it.
                if app.show_help {
                    app.show_help = false;
                    continue;
                }
                if app.mode == BrowserMode::Search {
                    if key.code == KeyCode::Enter {
                        // Filter mode narrows live and Enter just closes the
                        // box. Transcript mode (opened with `m`) commits an
                        // incremental full-text scan that Esc can cancel.
                        if app.search_scope == SearchScope::Transcripts
                            && callbacks.load_detail.is_some()
                        {
                            start_message_search(&mut app, true);
                        }
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
                if app.mode == BrowserMode::Rename {
                    handle_rename_key(&mut app, key, &mut callbacks.rename);
                    continue;
                }
                app.status = None;
                match key.code {
                    KeyCode::Char('q') => return Ok(None),
                    KeyCode::Esc => {
                        // With marks present, Esc first drops them; a second
                        // Esc quits. `q` always quits immediately.
                        if app.marked_count() > 0 {
                            app.marked_targets.clear();
                        } else {
                            return Ok(None);
                        }
                    }
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(None);
                    }
                    KeyCode::Up | KeyCode::Char('k') => app.previous(),
                    KeyCode::Down | KeyCode::Char('j') => app.next(),
                    KeyCode::PageUp => app.move_by(-10),
                    KeyCode::PageDown => app.move_by(10),
                    KeyCode::Home => app.first(),
                    KeyCode::End => app.last(),
                    KeyCode::Char(' ') => app.toggle_mark(),
                    KeyCode::Char('?') => app.show_help = true,
                    KeyCode::Char('/') => app.begin_filter_search(),
                    KeyCode::Char('m') => app.begin_transcript_search(),
                    KeyCode::Char(digit @ '1'..='4')
                        if !key
                            .modifiers
                            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                    {
                        if let Some(surface) = surface_for_digit(digit) {
                            return Ok(Some(SessionBrowserResult::Jump(surface)));
                        }
                    }
                    KeyCode::Char('g') => app.cycle_grouping(),
                    KeyCode::Char('r' | 'R') => {
                        return Ok(session_action_for_key(&app, key));
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
                    KeyCode::Char('f') => app.cycle_age(),
                    KeyCode::Char('t') => app.begin_rename(),
                    _ => {}
                }
            }
            Event::Paste(value) if app.mode == BrowserMode::Search => app.append_search(&value),
            Event::Paste(value) if app.mode == BrowserMode::Rename => app.append_rename(&value),
            Event::Mouse(mouse) if app.mode == BrowserMode::Browse => {
                handle_session_mouse(&mut app, mouse.kind);
            }
            _ => {}
        }
    }
}
