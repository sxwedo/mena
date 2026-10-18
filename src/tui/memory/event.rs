use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyModifiers, MouseEventKind};

use super::app::*;
use super::render::draw_memories;
use crate::memory::MemoryFile;
use crate::tui::common::{ManagedTerminal, Surface, is_key_press, surface_for_digit};

pub(crate) fn run_memory_browser(
    files: Vec<MemoryFile>,
    mut read: impl FnMut(&Path) -> Result<String>,
    mut delete: impl FnMut(&Path) -> Result<PathBuf>,
) -> Result<Option<Surface>> {
    let mut app = MemoriesApp::new(files);
    let mut terminal = Some(ManagedTerminal::enter_with_native_selection()?);
    let mut jump: Option<Surface> = None;

    loop {
        // Refresh the preview whenever the selection moves.
        let current = app.selected_file().map(|file| file.path.clone());
        if app.preview.as_ref().map(|(path, _)| path) != current.as_ref() {
            app.preview = current.map(|path| {
                let content = read(&path);
                (path, content.map_err(|error| format!("{error:#}")))
            });
            app.preview_scroll = 0;
        }

        terminal
            .as_mut()
            .expect("memory terminal is active")
            .terminal
            .draw(|frame| draw_memories(frame, &mut app))
            .context("failed to draw memory browser")?;

        if !event::poll(Duration::from_millis(90)).context("failed to poll terminal input")? {
            continue;
        }
        let input = event::read().context("failed to read terminal input")?;
        match input {
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollDown => {
                    if app.focus == MemoryFocus::List {
                        app.select_next();
                    } else {
                        app.preview_scroll = app.preview_scroll.saturating_add(2);
                    }
                }
                MouseEventKind::ScrollUp => {
                    if app.focus == MemoryFocus::List {
                        app.select_prev();
                    } else {
                        app.preview_scroll = app.preview_scroll.saturating_sub(2);
                    }
                }
                _ => {}
            },
            Event::Key(key) if is_key_press(&key) => {
                match handle_key(&mut app, &mut terminal, key, &mut delete)? {
                    Some(MemoryExit::Quit) => break,
                    Some(MemoryExit::Jump(surface)) => {
                        jump = Some(surface);
                        break;
                    }
                    None => {}
                }
            }
            _ => {}
        }
    }

    drop(terminal.take());
    Ok(jump)
}

/// Why the event loop stopped: plain quit or a digit-key surface jump.
enum MemoryExit {
    Quit,
    Jump(Surface),
}

/// Handle one key press; `Some` means the browser should exit.
fn handle_key(
    app: &mut MemoriesApp,
    terminal: &mut Option<ManagedTerminal>,
    key: crossterm::event::KeyEvent,
    delete: &mut impl FnMut(&Path) -> Result<PathBuf>,
) -> Result<Option<MemoryExit>> {
    // The `?` overlay explains the keymap; any key closes it.
    if app.show_help {
        app.show_help = false;
        return Ok(None);
    }
    if app.pending_delete.is_some() {
        match key.code {
            KeyCode::Char('y') => {
                let Some(file) = app.selected_file().cloned() else {
                    app.pending_delete = None;
                    return Ok(None);
                };
                match delete(&file.path) {
                    Ok(removed) => {
                        app.apply_deletion(&removed);
                        app.status = Some((format!("deleted {}", removed.display()), false));
                    }
                    Err(error) => {
                        app.pending_delete = None;
                        app.status = Some((format!("{error:#}"), true));
                    }
                }
            }
            KeyCode::Char('n') | KeyCode::Esc => app.pending_delete = None,
            _ => {}
        }
        return Ok(None);
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
        return Ok(None);
    }

    app.status = None;
    match key.code {
        KeyCode::Char('q') => return Ok(Some(MemoryExit::Quit)),
        KeyCode::Esc => {
            if app.focus == MemoryFocus::Preview {
                app.focus = MemoryFocus::List;
            } else {
                return Ok(Some(MemoryExit::Quit));
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.focus == MemoryFocus::List {
                app.select_prev();
            } else {
                app.preview_scroll = app.preview_scroll.saturating_sub(1);
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.focus == MemoryFocus::List {
                app.select_next();
            } else {
                app.preview_scroll = app.preview_scroll.saturating_add(1);
            }
        }
        KeyCode::Home => app.select_first(),
        KeyCode::End => app.select_last(),
        KeyCode::PageUp => app.preview_scroll = app.preview_scroll.saturating_sub(10),
        KeyCode::PageDown => app.preview_scroll = app.preview_scroll.saturating_add(10),
        KeyCode::Tab | KeyCode::Char('l') => {
            app.focus = match app.focus {
                MemoryFocus::List => MemoryFocus::Preview,
                MemoryFocus::Preview => MemoryFocus::List,
            };
        }
        KeyCode::Char('h') => app.focus = MemoryFocus::List,
        KeyCode::Char('/') => {
            app.is_searching = true;
            app.status = None;
        }
        KeyCode::Char('?') => app.show_help = true,
        KeyCode::Char('d') => app.begin_delete(),
        KeyCode::Char(digit @ '1'..='4')
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            if let Some(surface) = surface_for_digit(digit) {
                return Ok(Some(MemoryExit::Jump(surface)));
            }
        }
        KeyCode::Char('o') => {
            if let Some(file) = app.selected_file().cloned() {
                let result = {
                    drop(terminal.take());
                    let edited = crate::editor::edit_file_at_line(&file.path, 1);
                    *terminal = Some(ManagedTerminal::enter_with_native_selection()?);
                    edited
                };
                app.status = Some(match result {
                    Ok(()) => (format!("edited {}", file.name), false),
                    Err(error) => (format!("{error:#}"), true),
                });
                // The editor may have changed the file; reload the preview.
                app.preview = None;
            }
        }
        _ => {}
    }
    Ok(None)
}
