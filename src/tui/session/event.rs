use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind,
};

use super::app::*;
use crate::session::{AgentSession, DeletionSummary, DetailScope, SessionDetail};
use crate::tui::common::is_key_press;

pub(crate) fn session_action_for_key(
    app: &SessionsApp,
    key: KeyEvent,
) -> Option<SessionBrowserResult> {
    let session = app.selected_session()?.clone();
    match key.code {
        KeyCode::Char('r') => Some(SessionBrowserResult::Resume(session)),
        KeyCode::Char('R') => Some(SessionBrowserResult::ContinueWith(session)),
        _ => None,
    }
}

/// Advance an in-progress transcript search by one batch, polling (non-blocking)
/// for an Esc to cancel. Returns `Ok(true)` when a search is in progress and the
/// caller should `continue` the event loop (redraw next tick); `Ok(false)` when
/// no search is active so the caller proceeds to normal event reading.
pub(crate) fn pump_search(
    app: &mut SessionsApp,
    load_detail: Option<&mut DetailCallback<'_>>,
) -> Result<bool> {
    if app.search_in_progress.is_none() {
        return Ok(false);
    }
    let Some(load_detail) = load_detail else {
        return Ok(false);
    };
    step_message_search(app, load_detail);
    if app.search_in_progress.is_some()
        && event::poll(Duration::from_millis(0)).context("failed to poll input")?
        && matches!(
            event::read().context("failed to read terminal input")?,
            Event::Key(key) if is_key_press(&key) && key.code == KeyCode::Esc
        )
    {
        abort_message_search(app);
    }
    Ok(true)
}

/// Single-session actions stay locked while marks define a delete batch:
/// resume, continue-with, details, grouping, and filtering all act outside
/// the batch, so they explain themselves instead of firing.
pub(crate) fn is_batch_locked_key(app: &SessionsApp, key: &KeyEvent) -> bool {
    app.marked_count() > 0
        && matches!(
            key.code,
            KeyCode::Enter | KeyCode::Char('r' | 'R' | 'g' | 'i' | '/' | 't')
        )
        && !key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

pub(crate) fn handle_confirm_delete_key(
    app: &mut SessionsApp,
    key: KeyEvent,
    delete: &mut Option<DeleteCallback<'_>>,
) {
    match key.code {
        KeyCode::Char('y') => {
            let targets = std::mem::take(&mut app.confirm_delete_targets);
            if targets.is_empty() {
                app.mode = BrowserMode::Browse;
                return;
            }
            let Some(delete) = delete.as_deref_mut() else {
                app.mode = BrowserMode::Browse;
                return;
            };
            // Each deletion goes through the catalog seam, which re-checks
            // live-session protection fail-closed before removing anything.
            let mut removed = 0usize;
            let mut summary = DeletionSummary::default();
            for session in &targets {
                match delete(session) {
                    Ok(deletion) => {
                        summary.files += deletion.files;
                        summary.directories += deletion.directories;
                        summary.index_records += deletion.index_records;
                        app.apply_deletion(session);
                        removed += 1;
                    }
                    Err(error) => {
                        app.status = Some(StatusMessage::error(format!(
                            "Deleted {removed} of {} before failing: {error:#}",
                            targets.len()
                        )));
                        app.mode = BrowserMode::Browse;
                        return;
                    }
                }
            }
            app.status = Some(StatusMessage::success(if removed == 1 {
                format!(
                    "Permanently deleted {}: {} files, {} directories, {} index records",
                    targets[0].target(),
                    summary.files,
                    summary.directories,
                    summary.index_records
                )
            } else {
                format!(
                    "Permanently deleted {removed} sessions: {} files, {} directories, {} index records",
                    summary.files, summary.directories, summary.index_records
                )
            }));
            app.mode = BrowserMode::Browse;
        }
        KeyCode::Char('n') | KeyCode::Esc => {
            app.confirm_delete_targets.clear();
            app.mode = BrowserMode::Browse;
        }
        _ => {}
    }
}

pub(crate) fn handle_session_mouse(app: &mut SessionsApp, kind: MouseEventKind) {
    match kind {
        MouseEventKind::ScrollUp => app.move_by(-3),
        MouseEventKind::ScrollDown => app.move_by(3),
        _ => {}
    }
}

pub(crate) fn handle_search_key(app: &mut SessionsApp, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.query.clear();
            app.clear_message_search();
            app.mode = BrowserMode::Browse;
        }
        KeyCode::Backspace => {
            app.query.pop();
            // Editing the query invalidates a committed message search — the
            // user is refining live, so fall back to scalar filtering until the
            // next Enter.
            app.clear_message_search();
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
}

/// Begin an incremental full-text search over every session's transcript.
/// Returns `true` if a search is now running incrementally (the event loop
/// drives it with `step_message_search`). Returns `false` when there is no
/// transcript loader (e.g. `pick_session`) or the query is empty — in that case
/// it falls back to scalar-only filtering synchronously and no animation runs.
pub(crate) fn start_message_search(app: &mut SessionsApp, has_loader: bool) -> bool {
    if !has_loader {
        app.message_search = None;
        app.recompute_filter();
        return false;
    }
    let query = app.query.trim().to_ascii_lowercase();
    if query.is_empty() {
        app.clear_message_search();
        return false;
    }
    app.search_in_progress = Some(InProgressSearch {
        query,
        hits: BTreeSet::new(),
        errors: 0,
        cursor: 0,
        started: Instant::now(),
    });
    true
}

/// Advance an in-progress search by one session. Returns `true` once the whole
/// catalog has been scanned (the search is finalized and committed). No-op if no
/// search is in progress.
pub(crate) fn step_message_search<F>(app: &mut SessionsApp, load_detail: &mut F) -> bool
where
    F: FnMut(&AgentSession) -> Result<SessionDetail> + ?Sized,
{
    let Some(progress) = app.search_in_progress.as_mut() else {
        return true;
    };
    let query = progress.query.clone();
    let total = app.sessions.len();
    // Scan up to a small batch per tick so very small catalogs still animate a
    // frame or two but large ones don't spend forever idling between draws.
    while progress.cursor < total {
        let index = progress.cursor;
        progress.cursor += 1;
        let session = &app.sessions[index];
        // Scalar matches count as hits without paying for a transcript load.
        if session_matches(session, &query) {
            progress.hits.insert(index);
            continue;
        }
        match load_detail(session) {
            Ok(detail)
                if detail
                    .messages
                    .iter()
                    .any(|message| message.content.to_ascii_lowercase().contains(&query)) =>
            {
                progress.hits.insert(index);
            }
            Ok(_) => {}
            Err(_) => progress.errors += 1,
        }
        // Did real I/O this iteration — yield back to the loop for a redraw.
        return false;
    }
    // Cursor reached the end: commit.
    let InProgressSearch { hits, errors, .. } =
        app.search_in_progress.take().expect("in-progress search");
    let matched = hits.len();
    app.apply_message_search(hits);
    app.status = if errors > 0 {
        Some(StatusMessage::error(format!(
            "Searched transcripts — {matched} match{}, {errors} session{} failed to load",
            if matched == 1 { "" } else { "es" },
            if errors == 1 { "" } else { "s" }
        )))
    } else {
        Some(StatusMessage::success(format!(
            "Searched transcripts — {matched} match{}",
            if matched == 1 { "" } else { "es" }
        )))
    };
    true
}

/// Drop an in-progress search without committing partial results (Esc).
pub(crate) fn abort_message_search(app: &mut SessionsApp) {
    if app.search_in_progress.take().is_some() {
        app.status = Some(StatusMessage::error(
            "Transcript search cancelled".to_owned(),
        ));
    }
}

pub(crate) fn handle_rename_key(
    app: &mut SessionsApp,
    key: KeyEvent,
    rename: &mut Option<RenameCallback<'_>>,
) {
    match key.code {
        KeyCode::Esc => app.cancel_rename(),
        KeyCode::Enter => {
            let Some(rename) = rename.as_deref_mut() else {
                app.status = Some(StatusMessage::error("Rename is unavailable".to_owned()));
                app.cancel_rename();
                return;
            };
            app.commit_rename(rename);
        }
        KeyCode::Backspace => {
            app.status = None;
            app.rename_draft.pop();
        }
        KeyCode::Char(character)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            app.status = None;
            app.rename_draft.push(character);
        }
        _ => {}
    }
}

pub(crate) fn handle_detail_key(
    app: &mut SessionsApp,
    key: KeyEvent,
    export: Option<ExportCallback<'_>>,
    copy: Option<CopyCallback<'_>>,
) -> DetailAction {
    if key.code == KeyCode::Char('r') {
        return DetailAction::Resume;
    }
    if key.code == KeyCode::Char('R') {
        return DetailAction::ContinueWith;
    }
    if key.code == KeyCode::Char('t') {
        return DetailAction::Rename;
    }
    let scope = app.preview_scope;
    match key.code {
        // Vim-style in-detail search: `/` opens an incremental query, and a
        // committed search navigates with `n`/`N`. Esc clears the search
        // before it ever closes the popup.
        KeyCode::Char('/') => app.begin_detail_search(),
        KeyCode::Char('n') if app.detail_search.is_some() => {
            app.step_detail_match(true);
        }
        KeyCode::Char('N') if app.detail_search.is_some() => {
            app.step_detail_match(false);
        }
        KeyCode::Esc if app.detail_search.is_some() => app.cancel_detail_search(),
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('i' | 'q') => app.close_detail(),
        KeyCode::Char('c')
            if !key.modifiers.intersects(
                KeyModifiers::CONTROL
                    | KeyModifiers::ALT
                    | KeyModifiers::SUPER
                    | KeyModifiers::HYPER
                    | KeyModifiers::META,
            ) =>
        {
            let result = app
                .detail
                .as_ref()
                .zip(copy)
                .map(|(detail, copy)| copy(detail, scope));
            if let Some(result) = result {
                app.detail_status = Some(match result {
                    Ok(()) => {
                        StatusMessage::success(format!("Copied {} to clipboard", scope.label()))
                    }
                    Err(error) => StatusMessage::error(format!("Copy failed: {error:#}")),
                });
            }
        }
        KeyCode::Char('e') => {
            let result = app
                .detail
                .as_ref()
                .zip(export)
                .map(|(detail, export)| export(detail, scope));
            if let Some(result) = result {
                app.detail_status = Some(match result {
                    Ok(path) => StatusMessage::success(format!(
                        "Exported {}: {}",
                        scope.label(),
                        path.display()
                    )),
                    Err(error) => StatusMessage::error(format!("Export failed: {error:#}")),
                });
            }
        }
        KeyCode::Char('p') => app.set_preview_scope(DetailScope::Conversation),
        KeyCode::Char('P') => app.set_preview_scope(DetailScope::All),
        KeyCode::Up if key.modifiers.contains(KeyModifiers::SHIFT) => {
            app.jump_detail_primary(false);
        }
        KeyCode::Down if key.modifiers.contains(KeyModifiers::SHIFT) => {
            app.jump_detail_primary(true);
        }
        KeyCode::Up | KeyCode::Char('k') => app.scroll_detail(-1),
        KeyCode::Down | KeyCode::Char('j') => app.scroll_detail(1),
        KeyCode::PageUp => app.scroll_detail(-10),
        KeyCode::PageDown => app.scroll_detail(10),
        KeyCode::Home => app.detail_scroll = 0,
        KeyCode::End => app.detail_scroll = app.detail_max_scroll,
        _ => {}
    }
    DetailAction::Continue
}

pub(crate) fn handle_detail_event(
    app: &mut SessionsApp,
    input: &Event,
    export: Option<ExportCallback<'_>>,
    copy: Option<CopyCallback<'_>>,
) -> DetailAction {
    match input {
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            handle_detail_key(app, *key, export, copy)
        }
        Event::Mouse(mouse) => {
            match mouse.kind {
                MouseEventKind::ScrollUp => app.scroll_detail(-3),
                MouseEventKind::ScrollDown => app.scroll_detail(3),
                _ => {}
            }
            DetailAction::Continue
        }
        _ => DetailAction::Continue,
    }
}

pub(crate) fn handle_detail_browser_event(
    app: &mut SessionsApp,
    input: &Event,
    export: &mut Option<ExportCallback<'_>>,
    copy: &mut Option<CopyCallback<'_>>,
) -> DetailAction {
    match (export.as_mut(), copy.as_mut()) {
        (Some(export), Some(copy)) => {
            handle_detail_event(app, input, Some(&mut **export), Some(&mut **copy))
        }
        (Some(export), None) => handle_detail_event(app, input, Some(&mut **export), None),
        (None, Some(copy)) => handle_detail_event(app, input, None, Some(&mut **copy)),
        (None, None) => handle_detail_event(app, input, None, None),
    }
}

pub(crate) fn read_detail_event_batch(first: Event) -> Result<Vec<Event>> {
    const MAX_BATCH_SIZE: usize = 1_024;

    let mut events = vec![first];
    while events.len() < MAX_BATCH_SIZE
        && event::poll(Duration::ZERO).context("failed to poll queued detail input")?
    {
        events.push(event::read().context("failed to read queued detail input")?);
    }
    Ok(coalesce_detail_events(events))
}

pub(crate) fn coalesce_detail_events(events: impl IntoIterator<Item = Event>) -> Vec<Event> {
    let mut coalesced = Vec::new();
    for input in events {
        let direction = detail_scroll_direction(&input);
        if direction.is_some()
            && coalesced
                .last()
                .is_some_and(|previous| detail_scroll_direction(previous) == direction)
        {
            continue;
        }
        coalesced.push(input);
    }
    coalesced
}

fn detail_scroll_direction(input: &Event) -> Option<bool> {
    match input {
        Event::Mouse(mouse) if mouse.kind == MouseEventKind::ScrollUp => Some(false),
        Event::Mouse(mouse) if mouse.kind == MouseEventKind::ScrollDown => Some(true),
        Event::Key(KeyEvent {
            code: KeyCode::Up,
            kind: KeyEventKind::Press,
            ..
        }) => Some(false),
        Event::Key(KeyEvent {
            code: KeyCode::Down,
            kind: KeyEventKind::Press,
            ..
        }) => Some(true),
        _ => None,
    }
}

pub(crate) fn session_matches(session: &AgentSession, query: &str) -> bool {
    query.is_empty()
        || session.id.to_ascii_lowercase().contains(query)
        || session
            .kind
            .to_string()
            .to_ascii_lowercase()
            .contains(query)
        || session
            .title
            .as_deref()
            .is_some_and(|title| title.to_ascii_lowercase().contains(query))
        || session.project.as_ref().is_some_and(|project| {
            project
                .to_string_lossy()
                .to_ascii_lowercase()
                .contains(query)
        })
}
