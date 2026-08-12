use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseEventKind};

use super::app::*;
use super::render::draw_skills;
use crate::skill::{AgentSkill, SkillEntry};
use crate::tui::common::ManagedTerminal;

pub fn run_skill_browser(
    skills: Vec<AgentSkill>,
    load_entry: &mut impl FnMut(&AgentSkill, &std::path::Path) -> Result<SkillEntry>,
) -> Result<()> {
    let mut app = SkillsApp::new(skills);
    let mut terminal = ManagedTerminal::enter_with_native_selection()?;

    loop {
        // Refresh preview when selection changes (path-keyed cache)
        let current_path = app.selected_preview_path();
        if app.preview_path != current_path {
            app.preview_path.clone_from(&current_path);
            app.current_detail = None;
            app.preview_error = None;
            app.preview_scroll = 0;
        }

        if app.current_detail.is_none()
            && app.preview_error.is_none()
            && let Some(path) = current_path.as_deref()
            && let Some(skill_idx) = app
                .visible_rows
                .get(app.selected_index)
                .map(|row| match row {
                    SkillRow::Skill { skill_idx, .. } | SkillRow::Item { skill_idx, .. } => {
                        *skill_idx
                    }
                })
        {
            let skill = app.skills[skill_idx].clone();
            match load_entry(&skill, path) {
                Ok(entry) => {
                    app.cache_children(path.to_path_buf(), entry.children);
                    app.current_detail = Some(entry.detail);
                }
                Err(error) => app.preview_error = Some(format!("{error:#}")),
            }
        }

        terminal
            .terminal
            .draw(|frame| draw_skills(frame, &app))
            .context("failed to draw skill browser")?;

        if event::poll(Duration::from_millis(90)).context("failed to poll terminal input")? {
            let input = event::read().context("failed to read terminal input")?;
            match input {
                Event::Mouse(mouse_event) => match mouse_event.kind {
                    MouseEventKind::ScrollDown => {
                        if app.focus == SkillFocus::List && !app.full_screen_preview {
                            app.select_next();
                        } else {
                            app.preview_scroll = app.preview_scroll.saturating_add(2);
                        }
                    }
                    MouseEventKind::ScrollUp => {
                        if app.focus == SkillFocus::List && !app.full_screen_preview {
                            app.select_prev();
                        } else {
                            app.preview_scroll = app.preview_scroll.saturating_sub(2);
                        }
                    }
                    _ => {}
                },
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }

                    if app.is_searching {
                        match key.code {
                            KeyCode::Esc | KeyCode::Enter => {
                                app.is_searching = false;
                            }
                            KeyCode::Backspace => {
                                app.search_query.pop();
                                app.rebuild_rows();
                            }
                            KeyCode::Char(c) => {
                                app.search_query.push(c);
                                app.rebuild_rows();
                            }
                            _ => {}
                        }
                        continue;
                    }

                    match key.code {
                        KeyCode::Char('q') => {
                            if app.full_screen_preview {
                                app.full_screen_preview = false;
                            } else {
                                break;
                            }
                        }
                        KeyCode::Esc => {
                            if app.full_screen_preview {
                                app.full_screen_preview = false;
                            } else if app.focus == SkillFocus::Detail {
                                app.focus = SkillFocus::List;
                            } else {
                                break;
                            }
                        }
                        KeyCode::Char('s') => {
                            app.show_symlinks = !app.show_symlinks;
                            app.rebuild_rows();
                        }
                        KeyCode::Char('/') => {
                            app.is_searching = true;
                        }
                        KeyCode::Enter => {
                            if app.focus == SkillFocus::List && !app.full_screen_preview {
                                // Enter on list: toggle expand for directory skills
                                app.toggle_expand();
                            } else {
                                app.full_screen_preview = !app.full_screen_preview;
                            }
                        }
                        KeyCode::Tab | KeyCode::Char('l') => {
                            if !app.full_screen_preview {
                                app.focus = match app.focus {
                                    SkillFocus::List => SkillFocus::Detail,
                                    SkillFocus::Detail => SkillFocus::List,
                                };
                            }
                        }
                        KeyCode::Right => {
                            if app.focus == SkillFocus::List && !app.full_screen_preview {
                                app.toggle_expand();
                            } else if !app.full_screen_preview {
                                app.focus = SkillFocus::List;
                            }
                        }
                        KeyCode::Left | KeyCode::Char('h') => {
                            if app.focus == SkillFocus::List && !app.full_screen_preview {
                                app.collapse_current();
                            } else if !app.full_screen_preview {
                                app.focus = SkillFocus::List;
                            }
                        }
                        KeyCode::Char(' ') => {
                            if app.focus == SkillFocus::List && !app.full_screen_preview {
                                app.toggle_expand();
                            }
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if app.focus == SkillFocus::List && !app.full_screen_preview {
                                app.select_next();
                            } else {
                                app.preview_scroll = app.preview_scroll.saturating_add(1);
                            }
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            if app.focus == SkillFocus::List && !app.full_screen_preview {
                                app.select_prev();
                            } else {
                                app.preview_scroll = app.preview_scroll.saturating_sub(1);
                            }
                        }
                        KeyCode::PageDown => {
                            app.preview_scroll = app.preview_scroll.saturating_add(10);
                        }
                        KeyCode::PageUp => {
                            app.preview_scroll = app.preview_scroll.saturating_sub(10);
                        }
                        KeyCode::Char('o') => {
                            if let Some(path) = app.selected_open_path() {
                                open_in_editor(&path);
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        } else {
            app.marquee_offset = app.marquee_offset.wrapping_add(1);
        }
    }

    Ok(())
}
/// Open `path` in the best available editor, degrading gracefully:
/// $VISUAL → $EDITOR → `code` → `cursor` → `open` (macOS / xdg-open).
fn open_in_editor(path: &std::path::Path) {
    use std::process::Command;

    // Try GUI editors from env and common installs
    let candidates: &[&str] = &["VISUAL", "EDITOR"];
    for var in candidates {
        if let Ok(editor) = std::env::var(var)
            && !editor.is_empty()
        {
            let _ = Command::new(&editor).arg(path).spawn();
            return;
        }
    }

    for bin in &["code", "cursor"] {
        if Command::new(bin).arg("--version").output().is_ok() {
            let _ = Command::new(bin).arg(path).spawn();
            return;
        }
    }

    // macOS fallback: open with Finder / default app
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").arg(path).spawn();
    }

    // Linux fallback
    #[cfg(not(target_os = "macos"))]
    {
        let _ = Command::new("xdg-open").arg(path).spawn();
    }
}
