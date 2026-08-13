use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers, MouseEventKind};

use super::app::{McpApp, McpFocus};
use crate::mcp::McpRegistration;
use crate::tui::common::ManagedTerminal;
use crate::tui::common::is_key_press;

pub(crate) fn run_mcp_browser(
    registrations: Vec<McpRegistration>,
    mut probe: impl FnMut(&McpRegistration) -> Result<crate::mcp::McpDetail> + Send + 'static,
    mut locate: impl FnMut(&McpRegistration) -> Result<usize>,
    mut refresh: impl FnMut() -> Result<Vec<McpRegistration>>,
    mut delete: impl FnMut(&McpRegistration) -> Result<()>,
) -> Result<()> {
    let mut app = McpApp::new(registrations);
    let (request_tx, request_rx) = mpsc::sync_channel::<(usize, McpRegistration)>(1);
    let (result_tx, result_rx) = mpsc::channel();
    let mut worker = Some(
        thread::Builder::new()
            .name("mena-mcp-probe".to_owned())
            .spawn(move || {
                while let Ok((index, registration)) = request_rx.recv() {
                    let result = probe(&registration);
                    if result_tx.send((index, result)).is_err() {
                        break;
                    }
                }
            })
            .context("failed to start MCP probe worker")?,
    );
    let mut terminal = Some(ManagedTerminal::enter_with_native_selection()?);

    let loop_result = (|| -> Result<()> {
        loop {
            while let Ok((index, result)) = result_rx.try_recv() {
                app.finish_probe(index, result);
                if app.exit_after_probe {
                    break;
                }
            }
            if app.exit_after_probe && app.probe_in_progress.is_none() {
                break;
            }
            if app.probe_in_progress.is_some()
                && worker.as_ref().is_some_and(thread::JoinHandle::is_finished)
            {
                let index = app.probe_in_progress.expect("checked probe index");
                let _ = worker.take().expect("finished worker").join();
                app.finish_probe(index, Err(anyhow!("MCP probe worker stopped unexpectedly")));
                if app.exit_after_probe {
                    break;
                }
            }

            terminal
                .as_mut()
                .expect("MCP terminal is active")
                .terminal
                .draw(|frame| super::render::draw_mcp(frame, &mut app))
                .context("failed to draw MCP browser")?;

            if event::poll(Duration::from_millis(90)).context("failed to poll terminal input")? {
                let input = event::read().context("failed to read terminal input")?;
                match handle_event(&mut app, &input) {
                    McpAction::Continue => {}
                    McpAction::Quit if app.probe_in_progress.is_some() => {
                        app.exit_after_probe = true;
                    }
                    McpAction::Quit => break,
                    McpAction::Probe { index } => {
                        let registration = app.registrations[index].clone();
                        if let Err(error) = request_tx.send((index, registration)) {
                            app.finish_probe(
                                index,
                                Err(anyhow!("could not queue MCP probe: {error}")),
                            );
                        }
                    }
                    McpAction::Open { index } => {
                        let registration = app.registrations[index].clone();
                        let path = registration.source.clone();
                        let result = locate(&registration).and_then(|line| {
                            drop(terminal.take());
                            let opened = crate::editor::open_file_at_line(&path, line);
                            terminal = Some(ManagedTerminal::enter_with_native_selection()?);
                            opened.and_then(|()| {
                                refresh().context(
                                    "configuration opened but the catalog could not be refreshed",
                                )
                            })
                        });
                        app.finish_source_action(index, "open", "opened", result);
                    }
                    McpAction::Edit { index } => {
                        let registration = app.registrations[index].clone();
                        let path = registration.source.clone();
                        let result = locate(&registration).and_then(|line| {
                            drop(terminal.take());
                            let edited = crate::editor::edit_file_at_line(&path, line);
                            terminal = Some(ManagedTerminal::enter_with_native_selection()?);
                            edited.and_then(|()| {
                                refresh().context(
                                    "configuration edited but the catalog could not be refreshed",
                                )
                            })
                        });
                        app.finish_source_action(index, "edit", "edited", result);
                    }
                    McpAction::Delete { index } => {
                        let registration = app.registrations[index].clone();
                        let result = match delete(&registration) {
                            Ok(()) => refresh().context(
                                "registration was deleted but the catalog could not be refreshed",
                            ),
                            Err(error) => Err(error.context("MCP registration was not deleted")),
                        };
                        app.finish_delete(index, result);
                    }
                }
            } else {
                app.marquee_offset = app.marquee_offset.wrapping_add(1);
            }
        }
        Ok(())
    })();

    drop(terminal.take());
    drop(request_tx);
    let worker_result = worker.map_or_else(
        || Ok(()),
        |worker| {
            worker
                .join()
                .map_err(|_| anyhow!("MCP probe worker panicked"))
        },
    );
    loop_result.and(worker_result)
}

pub(crate) enum McpAction {
    Continue,
    Quit,
    Probe { index: usize },
    Open { index: usize },
    Edit { index: usize },
    Delete { index: usize },
}

pub(crate) fn handle_event(app: &mut McpApp, input: &Event) -> McpAction {
    match input {
        Event::Key(key) if is_key_press(key) => handle_key(app, *key),
        Event::Mouse(_) if app.pending_delete.is_some() => McpAction::Continue,
        Event::Mouse(mouse) => {
            match mouse.kind {
                MouseEventKind::ScrollDown => {
                    if app.focus == McpFocus::List && !app.full_screen_detail {
                        app.select_next();
                        app.select_next();
                    } else {
                        app.scroll_detail(2);
                    }
                }
                MouseEventKind::ScrollUp => {
                    if app.focus == McpFocus::List && !app.full_screen_detail {
                        app.select_previous();
                        app.select_previous();
                    } else {
                        app.scroll_detail(-2);
                    }
                }
                _ => {}
            }
            McpAction::Continue
        }
        _ => McpAction::Continue,
    }
}

pub(crate) fn handle_key(app: &mut McpApp, key: KeyEvent) -> McpAction {
    if let Some(index) = app.pending_delete {
        return match key.code {
            KeyCode::Char('y') => McpAction::Delete { index },
            KeyCode::Char('n') | KeyCode::Esc => {
                app.cancel_delete();
                McpAction::Continue
            }
            _ => McpAction::Continue,
        };
    }
    if app.is_searching {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => app.is_searching = false,
            KeyCode::Backspace => {
                app.query.pop();
                app.recompute_filter();
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                app.query.push(character);
                app.recompute_filter();
            }
            _ => {}
        }
        return McpAction::Continue;
    }

    match key.code {
        KeyCode::Char('q') => {
            if app.full_screen_detail {
                app.full_screen_detail = false;
                McpAction::Continue
            } else {
                McpAction::Quit
            }
        }
        KeyCode::Esc => {
            if app.full_screen_detail {
                app.full_screen_detail = false;
            } else if app.focus == McpFocus::Detail {
                app.focus = McpFocus::List;
            } else {
                return McpAction::Quit;
            }
            McpAction::Continue
        }
        KeyCode::Char('/') => {
            app.is_searching = true;
            McpAction::Continue
        }
        KeyCode::Char('p') => app
            .begin_probe()
            .map_or(McpAction::Continue, |index| McpAction::Probe { index }),
        KeyCode::Char('o') => app
            .selected_catalog_index()
            .map_or(McpAction::Continue, |index| McpAction::Open { index }),
        KeyCode::Char('e') => app
            .begin_external_edit()
            .map_or(McpAction::Continue, |index| McpAction::Edit { index }),
        KeyCode::Char('d') => {
            app.begin_delete();
            McpAction::Continue
        }
        KeyCode::Enter => {
            if app.selected_registration().is_some() {
                app.full_screen_detail = !app.full_screen_detail;
                if app.full_screen_detail {
                    app.focus = McpFocus::Detail;
                }
            }
            McpAction::Continue
        }
        KeyCode::Tab => {
            if !app.full_screen_detail {
                app.focus = match app.focus {
                    McpFocus::List => McpFocus::Detail,
                    McpFocus::Detail => McpFocus::List,
                };
            }
            McpAction::Continue
        }
        KeyCode::Right | KeyCode::Char('l') => {
            if !app.full_screen_detail {
                app.focus = McpFocus::Detail;
            }
            McpAction::Continue
        }
        KeyCode::Left | KeyCode::Char('h') => {
            if !app.full_screen_detail {
                app.focus = McpFocus::List;
            }
            McpAction::Continue
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.focus == McpFocus::List && !app.full_screen_detail {
                app.select_next();
            } else {
                app.scroll_detail(1);
            }
            McpAction::Continue
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.focus == McpFocus::List && !app.full_screen_detail {
                app.select_previous();
            } else {
                app.scroll_detail(-1);
            }
            McpAction::Continue
        }
        KeyCode::PageDown => {
            app.scroll_detail(10);
            McpAction::Continue
        }
        KeyCode::PageUp => {
            app.scroll_detail(-10);
            McpAction::Continue
        }
        KeyCode::Home => {
            if app.focus == McpFocus::List && !app.full_screen_detail {
                app.select_first();
            } else {
                app.detail_scroll = 0;
            }
            McpAction::Continue
        }
        KeyCode::End => {
            if app.focus == McpFocus::List && !app.full_screen_detail {
                app.select_last();
            } else {
                app.detail_scroll = app.detail_max_scroll;
            }
            McpAction::Continue
        }
        _ => McpAction::Continue,
    }
}

#[cfg(test)]
mod tests {
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

        let first = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
        );
        let second = handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
        );

        assert!(matches!(first, super::McpAction::Probe { index: 0 }));
        assert!(matches!(second, super::McpAction::Continue));
        assert_eq!(app.probe_in_progress, Some(0));
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
}
