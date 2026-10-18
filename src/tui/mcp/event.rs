use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers, MouseEventKind};

use super::app::{McpApp, McpFocus};
use crate::mcp::McpRegistration;
use crate::tui::common::{ManagedTerminal, Surface, is_key_press, surface_for_digit};

pub(crate) fn run_mcp_browser(
    registrations: Vec<McpRegistration>,
    mut probe: impl FnMut(&McpRegistration) -> Result<crate::mcp::McpDetail> + Send + 'static,
    mut locate: impl FnMut(&McpRegistration) -> Result<usize>,
    mut refresh: impl FnMut() -> Result<Vec<McpRegistration>>,
    mut delete: impl FnMut(&McpRegistration) -> Result<()>,
) -> Result<Option<Surface>> {
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

    let loop_result = (|| -> Result<Option<Surface>> {
        let mut jump: Option<Surface> = None;
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
                    McpAction::Jump(surface) => {
                        jump = Some(surface);
                        break;
                    }
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
        Ok(jump)
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
    loop_result.and_then(|jump| worker_result.map(|()| jump))
}

pub(crate) enum McpAction {
    Continue,
    Quit,
    Jump(Surface),
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
    // The `?` overlay explains the keymap; any key closes it.
    if app.show_help {
        app.show_help = false;
        return McpAction::Continue;
    }
    if app.pending_probe.is_some() {
        return match key.code {
            KeyCode::Char('y') => {
                app.pending_probe = None;
                app.begin_probe()
                    .map_or(McpAction::Continue, |index| McpAction::Probe { index })
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                app.cancel_probe();
                McpAction::Continue
            }
            _ => McpAction::Continue,
        };
    }
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
        KeyCode::Char('p') => {
            app.arm_probe();
            McpAction::Continue
        }
        KeyCode::Char('?') => {
            app.show_help = true;
            McpAction::Continue
        }
        KeyCode::Char(digit @ '1'..='4') => {
            surface_for_digit(digit).map_or(McpAction::Continue, McpAction::Jump)
        }
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
mod tests;
