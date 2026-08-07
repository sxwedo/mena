mod render;

use std::time::Duration;

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode};

pub(crate) use self::render::*;
use crate::AgentKind;
use crate::session::{AgentSession, NativeResumeCommand};
use crate::tui::common::{ManagedTerminal, is_key_press};

pub struct AgentLauncherItem {
    pub kind: AgentKind,
    pub installed: bool,
    pub session_count: usize,
    pub latest_session_id: Option<String>,
    pub latest_session_title: Option<String>,
}

pub fn select_and_launch_agent(
    custom: &std::collections::BTreeMap<String, crate::settings::CustomAgentSettings>,
    cwd_sessions: &[AgentSession],
) -> Result<Option<NativeResumeCommand>> {
    let kinds = AgentKind::all_kinds(custom);
    let mut items = Vec::new();
    for kind in kinds {
        let installed = kind.is_installed(custom);
        let matching: Vec<&AgentSession> = cwd_sessions.iter().filter(|s| s.kind == kind).collect();
        let session_count = matching.len();
        let latest = matching.first();
        items.push(AgentLauncherItem {
            kind,
            installed,
            session_count,
            latest_session_id: latest.map(|s| s.id.clone()),
            latest_session_title: latest.and_then(|s| s.title.clone()),
        });
    }
    items.sort_by_key(|item| !item.installed);

    let mut selected_index = 0;
    let mut tick: usize = 0;
    let mut terminal = ManagedTerminal::enter_with_native_selection()?;

    loop {
        terminal.terminal.draw(|frame| {
            draw_agent_selector(frame, &items, selected_index, tick);
        })?;

        tick = tick.wrapping_add(1);

        if !event::poll(Duration::from_millis(40)).context("failed to poll terminal input")? {
            continue;
        }

        let input = event::read().context("failed to read terminal input")?;
        if let Event::Key(key) = input
            && is_key_press(&key)
        {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    selected_index = selected_index.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if !items.is_empty() {
                        selected_index = (selected_index + 1).min(items.len() - 1);
                    }
                }
                KeyCode::Char('n') => {
                    if let Some(item) = items.get(selected_index) {
                        return crate::controller::fresh_launch_spec(&item.kind, custom).map(Some);
                    }
                }
                KeyCode::Char('r') => {
                    if let Some(item) = items.get(selected_index) {
                        if let Some(ref id) = item.latest_session_id {
                            return crate::controller::resume_launch_spec(&item.kind, id, custom)
                                .map(Some);
                        }
                        return crate::controller::fresh_launch_spec(&item.kind, custom).map(Some);
                    }
                }
                KeyCode::Enter => {
                    if let Some(item) = items.get(selected_index) {
                        if !item.installed {
                            drop(terminal);
                            return crate::controller::open_url(item.kind.homepage_url())
                                .map(|()| None);
                        }
                        let matching: Vec<&AgentSession> = cwd_sessions
                            .iter()
                            .filter(|s| s.kind == item.kind)
                            .collect();
                        if matching.is_empty() {
                            return crate::controller::fresh_launch_spec(&item.kind, custom)
                                .map(Some);
                        }
                        drop(terminal);
                        return select_launch_mode_for_agent(&item.kind, custom, &matching);
                    }
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    return Ok(None);
                }
                _ => {}
            }
        }
    }
}

pub fn select_launch_mode_for_agent(
    kind: &AgentKind,
    custom: &std::collections::BTreeMap<String, crate::settings::CustomAgentSettings>,
    matching_sessions: &[&AgentSession],
) -> Result<Option<NativeResumeCommand>> {
    #[derive(Clone)]
    enum ModeOption {
        Fresh,
        ResumeSession(String),
    }

    let mut options = vec![(ModeOption::Fresh, "✨ Start New Session".to_owned())];
    for session in matching_sessions {
        let title = session.title.as_deref().unwrap_or("Untitled session");
        let label = format!("⚡ Resume: {} ({title})", session.id);
        options.push((ModeOption::ResumeSession(session.id.clone()), label));
    }

    let mut selected_index = 0;
    let mut tick: usize = 0;
    let mut terminal = ManagedTerminal::enter_with_native_selection()?;

    loop {
        terminal.terminal.draw(|frame| {
            draw_mode_selector(frame, kind, &options, selected_index, tick);
        })?;

        tick = tick.wrapping_add(1);

        if !event::poll(Duration::from_millis(40)).context("failed to poll terminal input")? {
            continue;
        }

        let input = event::read().context("failed to read terminal input")?;
        if let Event::Key(key) = input
            && is_key_press(&key)
        {
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    selected_index = selected_index.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if !options.is_empty() {
                        selected_index = (selected_index + 1).min(options.len() - 1);
                    }
                }
                KeyCode::Enter => {
                    if let Some((opt, _)) = options.get(selected_index) {
                        match opt {
                            ModeOption::Fresh => {
                                return crate::controller::fresh_launch_spec(kind, custom)
                                    .map(Some);
                            }
                            ModeOption::ResumeSession(id) => {
                                return crate::controller::resume_launch_spec(kind, id, custom)
                                    .map(Some);
                            }
                        }
                    }
                }
                KeyCode::Esc | KeyCode::Char('q') => {
                    return Ok(None);
                }
                _ => {}
            }
        }
    }
}
