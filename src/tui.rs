use std::collections::BTreeSet;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind,
};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Wrap};
use ratatui::{DefaultTerminal, Frame};

use crate::session::{
    AgentSession, DeletionSummary, DetailScope, SessionDetail, SessionMessageKind,
};
use crate::settings::{ConfigColor, SessionDetailColorSettings};
use crate::view::{
    TOOL_TOKEN_ACCOUNTING_NOTE, format_duration, format_metric_error, format_model_usage_summary,
    format_response_header_metrics, format_response_summary, format_token_breakdown,
    format_tool_summary,
};

const ACCENT: Color = Color::Cyan;
const MUTED: Color = Color::DarkGray;
#[allow(dead_code)]
const METADATA_KEY: Color = Color::LightMagenta;

#[derive(Debug, Clone, Copy)]
struct SessionDetailTheme {
    border: Color,
    popup_title: Color,
    metadata_key: Color,
    metadata_value: Color,
    conversation_header: Color,
    empty_text: Color,
    status_success: Color,
    status_error: Color,
    footer_key: Color,
    footer_text: Color,
    footer_separator: Color,
    user_header: Color,
    user_content: Color,
    assistant_header: Color,
    assistant_content: Color,
    skill_header: Color,
    skill_content: Color,
    tool_call_header: Color,
    tool_call_content: Color,
    tool_result_header: Color,
    tool_result_content: Color,
    system_header: Color,
    system_content: Color,
    error_header: Color,
    error_content: Color,
}

impl From<&SessionDetailColorSettings> for SessionDetailTheme {
    fn from(colors: &SessionDetailColorSettings) -> Self {
        Self {
            border: configured_color(colors.border),
            popup_title: configured_color(colors.popup_title),
            metadata_key: configured_color(colors.metadata_key),
            metadata_value: configured_color(colors.metadata_value),
            conversation_header: configured_color(colors.conversation_header),
            empty_text: configured_color(colors.empty_text),
            status_success: configured_color(colors.status_success),
            status_error: configured_color(colors.status_error),
            footer_key: configured_color(colors.footer_key),
            footer_text: configured_color(colors.footer_text),
            footer_separator: configured_color(colors.footer_separator),
            user_header: configured_color(colors.user_header),
            user_content: configured_color(colors.user_content),
            assistant_header: configured_color(colors.assistant_header),
            assistant_content: configured_color(colors.assistant_content),
            skill_header: configured_color(colors.skill_header),
            skill_content: configured_color(colors.skill_content),
            tool_call_header: configured_color(colors.tool_call_header),
            tool_call_content: configured_color(colors.tool_call_content),
            tool_result_header: configured_color(colors.tool_result_header),
            tool_result_content: configured_color(colors.tool_result_content),
            system_header: configured_color(colors.system_header),
            system_content: configured_color(colors.system_content),
            error_header: configured_color(colors.error_header),
            error_content: configured_color(colors.error_content),
        }
    }
}

impl Default for SessionDetailTheme {
    fn default() -> Self {
        Self::from(&SessionDetailColorSettings::default())
    }
}

const fn configured_color(color: ConfigColor) -> Color {
    match color {
        ConfigColor::Reset => Color::Reset,
        ConfigColor::Black => Color::Black,
        ConfigColor::Red => Color::Red,
        ConfigColor::Green => Color::Green,
        ConfigColor::Yellow => Color::Yellow,
        ConfigColor::Blue => Color::Blue,
        ConfigColor::Magenta => Color::Magenta,
        ConfigColor::Cyan => Color::Cyan,
        ConfigColor::Gray => Color::Gray,
        ConfigColor::DarkGray => Color::DarkGray,
        ConfigColor::LightRed => Color::LightRed,
        ConfigColor::LightGreen => Color::LightGreen,
        ConfigColor::LightYellow => Color::LightYellow,
        ConfigColor::LightBlue => Color::LightBlue,
        ConfigColor::LightMagenta => Color::LightMagenta,
        ConfigColor::LightCyan => Color::LightCyan,
        ConfigColor::White => Color::White,
        ConfigColor::Indexed(index) => Color::Indexed(index),
        ConfigColor::Rgb(red, green, blue) => Color::Rgb(red, green, blue),
    }
}
type DeleteCallback<'a> = &'a mut dyn FnMut(&AgentSession) -> Result<DeletionSummary>;
type DetailCallback<'a> = &'a mut dyn FnMut(&AgentSession) -> Result<SessionDetail>;
type ExportCallback<'a> = &'a mut dyn FnMut(&SessionDetail, DetailScope) -> Result<PathBuf>;
type CopyCallback<'a> = &'a mut dyn FnMut(&SessionDetail, DetailScope) -> Result<()>;

#[derive(Default)]
struct SessionBrowserCallbacks<'a> {
    load_detail: Option<DetailCallback<'a>>,
    export: Option<ExportCallback<'a>>,
    copy: Option<CopyCallback<'a>>,
    delete: Option<DeleteCallback<'a>>,
}

pub fn manage_skills(
    skills: Vec<AgentSkill>,
    mut load_detail: impl FnMut(&AgentSkill) -> Result<SkillDetail>,
) -> Result<()> {
    run_skill_browser(skills, &mut load_detail)
}

pub fn manage_sessions(
    sessions: Vec<AgentSession>,
    active_targets: BTreeSet<String>,
    detail_colors: &SessionDetailColorSettings,
    mut load_detail: impl FnMut(&AgentSession) -> Result<SessionDetail>,
    mut export: impl FnMut(&SessionDetail, DetailScope) -> Result<PathBuf>,
    mut copy: impl FnMut(&SessionDetail, DetailScope) -> Result<()>,
    mut delete: impl FnMut(&AgentSession) -> Result<DeletionSummary>,
) -> Result<Option<AgentSession>> {
    run_session_browser(
        sessions,
        active_targets,
        SessionDetailTheme::from(detail_colors),
        SessionBrowserCallbacks {
            load_detail: Some(&mut load_detail),
            export: Some(&mut export),
            copy: Some(&mut copy),
            delete: Some(&mut delete),
        },
    )
}

/// Advance an in-progress transcript search by one batch, polling (non-blocking)
/// for an Esc to cancel. Returns `Ok(true)` when a search is in progress and the
/// caller should `continue` the event loop (redraw next tick); `Ok(false)` when
/// no search is active so the caller proceeds to normal event reading.
fn pump_search(
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

#[allow(clippy::too_many_lines)]
fn run_session_browser(
    sessions: Vec<AgentSession>,
    active_targets: BTreeSet<String>,
    detail_theme: SessionDetailTheme,
    mut callbacks: SessionBrowserCallbacks<'_>,
) -> Result<Option<AgentSession>> {
    let mut app = SessionsApp::new_with_detail_theme(sessions, active_targets, detail_theme);
    let mut terminal = ManagedTerminal::enter_with_native_selection()?;
    loop {
        terminal
            .terminal
            .draw(|frame| draw_sessions(frame, &mut app))
            .context("failed to draw session browser")?;
        // While a transcript search runs incrementally, drive it one batch per
        // frame instead of blocking on event::read. Esc cancels; any other key
        // is swallowed until the scan finishes. Each tick redraws the spinner.
        if pump_search(&mut app, callbacks.load_detail.as_mut())
            .context("failed to advance transcript search")?
        {
            continue;
        }
        let input = event::read().context("failed to read terminal input")?;
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

fn handle_confirm_delete_key(
    app: &mut SessionsApp,
    key: KeyEvent,
    delete: &mut Option<DeleteCallback<'_>>,
) {
    match key.code {
        KeyCode::Char('y') => {
            if let Some(session) = app.selected_session().cloned()
                && let Some(delete) = delete.as_deref_mut()
            {
                match delete(&session) {
                    Ok(summary) => app.deleted(&session, summary),
                    Err(error) => {
                        app.status =
                            Some(StatusMessage::error(format!("Delete failed: {error:#}")));
                        app.mode = BrowserMode::Browse;
                    }
                }
            }
        }
        KeyCode::Char('n') | KeyCode::Esc => app.mode = BrowserMode::Browse,
        _ => {}
    }
}

fn handle_session_mouse(app: &mut SessionsApp, kind: MouseEventKind) {
    match kind {
        MouseEventKind::ScrollUp => app.move_by(-3),
        MouseEventKind::ScrollDown => app.move_by(3),
        _ => {}
    }
}

fn handle_search_key(app: &mut SessionsApp, key: KeyEvent) {
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
fn start_message_search(app: &mut SessionsApp, has_loader: bool) -> bool {
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
fn step_message_search<F>(app: &mut SessionsApp, load_detail: &mut F) -> bool
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
fn abort_message_search(app: &mut SessionsApp) {
    if app.search_in_progress.take().is_some() {
        app.status = Some(StatusMessage::error(
            "Transcript search cancelled".to_owned(),
        ));
    }
}

struct ManagedTerminal {
    terminal: DefaultTerminal,
    alternate_scroll: bool,
}

impl ManagedTerminal {
    #[allow(dead_code)]
    fn enter() -> Result<Self> {
        Self::enter_internal(false)
    }

    fn enter_with_native_selection() -> Result<Self> {
        Self::enter_internal(true)
    }

    fn enter_internal(alternate_scroll: bool) -> Result<Self> {
        let mut terminal = ratatui::try_init().context("failed to initialize terminal UI")?;
        if alternate_scroll
            && let Err(error) = configure_alternate_scroll(&mut std::io::stdout(), true)
        {
            let _ = ratatui::try_restore();
            return Err(error).context("failed to enable terminal alternate scrolling");
        }
        if let Err(error) = terminal.hide_cursor() {
            if alternate_scroll {
                let _ = configure_alternate_scroll(&mut std::io::stdout(), false);
            }
            let _ = ratatui::try_restore();
            return Err(error).context("failed to hide terminal cursor");
        }
        Ok(Self {
            terminal,
            alternate_scroll,
        })
    }
}

fn configure_alternate_scroll(writer: &mut impl Write, enabled: bool) -> io::Result<()> {
    writer.write_all(if enabled {
        b"\x1b[?1007h"
    } else {
        b"\x1b[?1007l"
    })?;
    writer.flush()
}

impl Drop for ManagedTerminal {
    fn drop(&mut self) {
        let _ = self.terminal.show_cursor();
        if self.alternate_scroll {
            let _ = configure_alternate_scroll(&mut std::io::stdout(), false);
        }
        let _ = ratatui::try_restore();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrowserMode {
    Browse,
    Search,
    Detail,
    ConfirmDelete,
}

/// How the session list is grouped in the table. `Flat` is the single
/// flat list; `Project` inserts selectable header rows per project, allows
/// collapsing/expanding groups via Enter, and keeps original within-group order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Grouping {
    #[default]
    Flat,
    Project,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DisplayRow {
    GroupHeader {
        project: String,
        count: usize,
        collapsed: bool,
    },
    Session {
        session_index: usize,
    },
}

impl Grouping {
    /// Cycle to the next grouping mode (bound to a key in Browse mode).
    const fn cycle(self) -> Self {
        match self {
            Self::Flat => Self::Project,
            Self::Project => Self::Flat,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Flat => "flat",
            Self::Project => "by project",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DetailAction {
    Continue,
    Resume,
}

/// A full-text message search running incrementally so the UI can show
/// progress and stay responsive (Esc cancels) while transcripts load one by
/// one. Lives on `SessionsApp::search_in_progress`; the event loop drives one
/// `step_message_search` per tick.
#[derive(Debug)]
struct InProgressSearch {
    /// Lowercased committed query, captured at submit time.
    query: String,
    /// Session indices whose transcripts match the query, accumulated as we go.
    hits: BTreeSet<usize>,
    /// Number of sessions whose transcripts failed to load.
    errors: u32,
    /// Index (into `SessionsApp::sessions`) of the next session to scan.
    cursor: usize,
    /// When the search started, used to animate the spinner between ticks.
    started: Instant,
}

#[derive(Debug)]
struct SessionsApp {
    sessions: Vec<AgentSession>,
    filtered: Vec<usize>,
    active_targets: BTreeSet<String>,
    table_state: TableState,
    query: String,
    mode: BrowserMode,
    detail_theme: SessionDetailTheme,
    detail: Option<SessionDetail>,
    detail_scroll: usize,
    detail_max_scroll: usize,
    detail_primary_offsets: Vec<usize>,
    detail_layout: Option<DetailLayoutCache>,
    detail_status: Option<StatusMessage>,
    status: Option<StatusMessage>,
    /// Session indices whose messages contain the committed query. Populated only
    /// after the user presses Enter in Search mode, so live per-keystroke filtering
    /// stays on the cheap scalar fields while full-text search runs once on submit.
    message_search: Option<BTreeSet<usize>>,
    /// A message search currently running incrementally. `Some` only between
    /// pressing Enter (commit) and the scan finishing (or Esc cancelling).
    search_in_progress: Option<InProgressSearch>,
    /// Whether the table renders flat or with per-project group headers.
    grouping: Grouping,
    /// Set of project keys currently collapsed in Project grouping mode.
    collapsed_projects: BTreeSet<String>,
    /// Which messages the detail preview shows. Defaults to `Conversation`
    /// (user/assistant only); toggled with `p`/`Shift+P` in detail mode.
    preview_scope: DetailScope,
}

impl SessionsApp {
    #[cfg(test)]
    fn new(sessions: Vec<AgentSession>, active_targets: BTreeSet<String>) -> Self {
        Self::new_with_detail_theme(sessions, active_targets, SessionDetailTheme::default())
    }

    fn new_with_detail_theme(
        sessions: Vec<AgentSession>,
        active_targets: BTreeSet<String>,
        detail_theme: SessionDetailTheme,
    ) -> Self {
        let mut app = Self {
            sessions,
            filtered: Vec::new(),
            active_targets,
            table_state: TableState::default(),
            query: String::new(),
            mode: BrowserMode::Browse,
            detail_theme,
            detail: None,
            detail_scroll: 0,
            detail_max_scroll: 0,
            detail_primary_offsets: Vec::new(),
            detail_layout: None,
            detail_status: None,
            status: None,
            message_search: None,
            search_in_progress: None,
            grouping: Grouping::Flat,
            collapsed_projects: BTreeSet::default(),
            preview_scope: DetailScope::Conversation,
        };
        app.recompute_filter();
        app
    }

    fn recompute_filter(&mut self) {
        let query = self.query.to_ascii_lowercase();
        let scalar_hits = |session: &AgentSession| session_matches(session, &query);
        let message_hits = self.message_search.as_ref();
        self.filtered = self
            .sessions
            .iter()
            .enumerate()
            .filter(|(index, session)| {
                scalar_hits(session) || message_hits.is_some_and(|hits| hits.contains(index))
            })
            .map(|(index, _)| index)
            .collect();
        self.clamp_selection();
    }

    fn clamp_selection(&mut self) {
        let len = self.display_rows().len();
        let selected = self.table_state.selected().unwrap_or_default();
        self.table_state
            .select(len.checked_sub(1).map(|last| selected.min(last)));
    }

    fn display_rows(&self) -> Vec<DisplayRow> {
        match self.grouping {
            Grouping::Flat => self
                .filtered
                .iter()
                .map(|&session_index| DisplayRow::Session { session_index })
                .collect(),
            Grouping::Project => {
                let mut seen: Vec<String> = Vec::new();
                for &session_index in &self.filtered {
                    let key = session_project_label(&self.sessions[session_index]);
                    if !seen.contains(&key) {
                        seen.push(key);
                    }
                }
                let mut rows = Vec::new();
                for key in seen {
                    let count = self
                        .filtered
                        .iter()
                        .filter(|&&idx| session_project_label(&self.sessions[idx]) == key)
                        .count();
                    let collapsed = self.collapsed_projects.contains(&key);
                    rows.push(DisplayRow::GroupHeader {
                        project: key.clone(),
                        count,
                        collapsed,
                    });
                    if !collapsed {
                        for &session_index in &self.filtered {
                            if session_project_label(&self.sessions[session_index]) == key {
                                rows.push(DisplayRow::Session { session_index });
                            }
                        }
                    }
                }
                rows
            }
        }
    }

    fn selected_row(&self) -> Option<DisplayRow> {
        let rows = self.display_rows();
        let selected = self.table_state.selected()?;
        rows.get(selected).cloned()
    }

    fn toggle_project_collapse(&mut self, project: &str) {
        if self.collapsed_projects.contains(project) {
            self.collapsed_projects.remove(project);
        } else {
            self.collapsed_projects.insert(project.to_owned());
        }
        self.clamp_selection();
    }

    fn append_search(&mut self, value: &str) {
        self.query.push_str(value);
        self.recompute_filter();
    }

    fn collapse_all_projects(&mut self) {
        self.collapsed_projects.clear();
        for &session_index in &self.filtered {
            let key = session_project_label(&self.sessions[session_index]);
            self.collapsed_projects.insert(key);
        }
    }

    /// Toggle the list grouping (flat ↔ by project).
    fn cycle_grouping(&mut self) {
        let current_session_index = match self.selected_row() {
            Some(DisplayRow::Session { session_index }) => Some(session_index),
            _ => None,
        };
        self.grouping = self.grouping.cycle();
        if self.grouping == Grouping::Project {
            self.collapse_all_projects();
        }
        let display_rows = self.display_rows();
        if let Some(target_index) = current_session_index {
            if let Some(new_pos) = display_rows.iter().position(|r| {
                matches!(r, DisplayRow::Session { session_index } if *session_index == target_index)
            }) {
                self.table_state.select(Some(new_pos));
            } else {
                self.clamp_selection();
            }
        } else {
            self.clamp_selection();
        }
        self.status = Some(StatusMessage::success(format!(
            "Grouped: {}",
            self.grouping.label(),
        )));
    }

    /// Commit a full-text search over session transcripts. `hits` are the indices
    /// of sessions whose messages contain the (already lowercased) query. Clears
    /// any prior message search so a fresh Enter re-runs against current results.
    fn apply_message_search(&mut self, hits: BTreeSet<usize>) {
        self.message_search = Some(hits);
        self.recompute_filter();
    }

    /// Drop any committed message search, returning to scalar-only filtering.
    fn clear_message_search(&mut self) {
        if self.message_search.take().is_some() {
            self.recompute_filter();
        }
    }

    fn selected_session(&self) -> Option<&AgentSession> {
        match self.selected_row()? {
            DisplayRow::Session { session_index } => self.sessions.get(session_index),
            DisplayRow::GroupHeader { .. } => None,
        }
    }

    fn previous(&mut self) {
        self.move_by(-1);
    }

    fn next(&mut self) {
        self.move_by(1);
    }

    fn move_by(&mut self, amount: isize) {
        let len = self.display_rows().len();
        if len == 0 {
            self.table_state.select(None);
            return;
        }
        let selected = self.table_state.selected().unwrap_or_default();
        let selected = selected.saturating_add_signed(amount).min(len - 1);
        self.table_state.select(Some(selected));
    }

    fn first(&mut self) {
        let len = self.display_rows().len();
        self.table_state.select((len > 0).then_some(0));
    }

    fn last(&mut self) {
        let len = self.display_rows().len();
        self.table_state.select(len.checked_sub(1));
    }

    fn open_detail(&mut self, detail: SessionDetail) {
        self.detail = Some(detail);
        self.detail_scroll = 0;
        self.detail_max_scroll = 0;
        self.detail_primary_offsets.clear();
        self.detail_layout = None;
        self.detail_status = None;
        // Each freshly opened session starts in the conversation-only preview;
        // the user can press Shift+P to reveal tool/system messages.
        self.preview_scope = DetailScope::Conversation;
        self.mode = BrowserMode::Detail;
    }

    /// Switch the detail preview scope. Invalidates the cached layout so the
    /// next redraw reflows for the new message set, and clamps scroll.
    fn set_preview_scope(&mut self, scope: DetailScope) {
        if self.preview_scope == scope {
            return;
        }
        self.preview_scope = scope;
        self.detail_scroll = 0;
        self.detail_max_scroll = 0;
        self.detail_primary_offsets.clear();
        self.detail_layout = None;
    }

    fn close_detail(&mut self) {
        self.detail = None;
        self.detail_scroll = 0;
        self.detail_max_scroll = 0;
        self.detail_primary_offsets.clear();
        self.detail_layout = None;
        self.detail_status = None;
        self.mode = BrowserMode::Browse;
    }

    fn scroll_detail(&mut self, amount: isize) {
        self.detail_scroll = self
            .detail_scroll
            .saturating_add_signed(amount)
            .min(self.detail_max_scroll);
    }

    fn jump_detail_primary(&mut self, forward: bool) {
        let target = if forward {
            self.detail_primary_offsets
                .iter()
                .copied()
                .find(|offset| *offset > self.detail_scroll)
        } else {
            self.detail_primary_offsets
                .iter()
                .copied()
                .rev()
                .find(|offset| *offset < self.detail_scroll)
        };
        if let Some(target) = target {
            self.detail_scroll = target;
        }
    }

    fn request_delete(&mut self) {
        let Some(session) = self.selected_session() else {
            return;
        };
        if self.active_targets.contains(&session.target()) {
            self.status = Some(StatusMessage::error(
                "Cannot delete a session that may be attached to a running agent".to_owned(),
            ));
        } else {
            self.mode = BrowserMode::ConfirmDelete;
        }
    }

    fn deleted(&mut self, deleted: &AgentSession, summary: DeletionSummary) {
        let target = deleted.target();
        self.sessions
            .retain(|session| session.kind != deleted.kind || session.id != deleted.id);
        self.status = Some(StatusMessage::success(format!(
            "Permanently deleted {target}: {} files, {} directories, {} index records",
            summary.files, summary.directories, summary.index_records
        )));
        self.mode = BrowserMode::Browse;
        self.recompute_filter();
    }
}

fn handle_detail_key(
    app: &mut SessionsApp,
    key: KeyEvent,
    export: Option<ExportCallback<'_>>,
    copy: Option<CopyCallback<'_>>,
) -> DetailAction {
    if key.code == KeyCode::Char('r') {
        return DetailAction::Resume;
    }
    let scope = app.preview_scope;
    match key.code {
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

fn handle_detail_event(
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

fn handle_detail_browser_event(
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

fn read_detail_event_batch(first: Event) -> Result<Vec<Event>> {
    const MAX_BATCH_SIZE: usize = 1_024;

    let mut events = vec![first];
    while events.len() < MAX_BATCH_SIZE
        && event::poll(Duration::ZERO).context("failed to poll queued detail input")?
    {
        events.push(event::read().context("failed to read queued detail input")?);
    }
    Ok(coalesce_detail_events(events))
}

fn coalesce_detail_events(events: impl IntoIterator<Item = Event>) -> Vec<Event> {
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

#[derive(Debug)]
struct StatusMessage {
    text: String,
    style: Style,
    is_error: bool,
}

impl StatusMessage {
    fn success(text: String) -> Self {
        Self {
            text,
            style: Style::default().fg(Color::Green),
            is_error: false,
        }
    }

    fn error(text: String) -> Self {
        Self {
            text,
            style: Style::default().fg(Color::Red),
            is_error: true,
        }
    }
}

fn session_matches(session: &AgentSession, query: &str) -> bool {
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

const fn is_key_press(key: &KeyEvent) -> bool {
    matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
}

fn draw_sessions(frame: &mut Frame<'_>, app: &mut SessionsApp) {
    let areas = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(5),
        Constraint::Length(1),
    ])
    .split(frame.area());

    render_session_search(frame, areas[0], app);
    render_session_table_widget(frame, areas[1], app);
    render_session_footer(frame, areas[2], app);
    render_session_detail_popup(frame, app);
    render_delete_confirmation(frame, app);
}

fn render_session_search(frame: &mut Frame<'_>, area: Rect, app: &SessionsApp) {
    let (style, title) = if app.mode == BrowserMode::Search {
        (
            Style::default().fg(Color::Black).bg(ACCENT),
            " Search — type to filter, Enter apply, Esc clear ",
        )
    } else {
        (Style::default().fg(MUTED), " Search — press / to filter ")
    };
    let query = if app.query.is_empty() {
        "All sessions"
    } else {
        &app.query
    };
    frame.render_widget(
        Paragraph::new(query)
            .style(style)
            .block(Block::new().borders(Borders::ALL).title(title)),
        area,
    );
}

fn format_project_display_path(path_str: &str, max_len: usize) -> String {
    let mut formatted = path_str.to_owned();
    if let Some(home) = dirs::home_dir() {
        let home_str = home.to_string_lossy();
        if formatted.starts_with(home_str.as_ref()) {
            formatted = format!("~{}", &formatted[home_str.len()..]);
        }
    }
    if formatted.chars().count() > max_len && max_len > 12 {
        let components: Vec<&str> = formatted.split('/').collect();
        if components.len() > 3 {
            let prefix = components[0];
            let first_dir = components[1];
            let last_dir = components.last().copied().unwrap_or("");
            let candidate = format!("{prefix}/{first_dir}/.../{last_dir}");
            if candidate.chars().count() <= max_len {
                return candidate;
            }
        }
        let truncated: String = formatted.chars().take(max_len - 3).collect();
        format!("{truncated}...")
    } else {
        formatted
    }
}
fn render_session_table_widget(frame: &mut Frame<'_>, area: Rect, app: &mut SessionsApp) {
    let columns = session_columns(area.width);
    let header = Row::new(columns.iter().map(|column| Cell::from(column.label)))
        .style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .bottom_margin(1);

    let display_rows = app.display_rows();
    let mut rows: Vec<Row<'_>> = Vec::with_capacity(display_rows.len());
    let session_row = |session_index: usize| {
        let session = &app.sessions[session_index];
        Row::new(
            columns
                .iter()
                .map(|column| session_cell(session, column.kind, app)),
        )
    };

    for item in &display_rows {
        match item {
            DisplayRow::GroupHeader {
                project,
                count,
                collapsed,
            } => {
                let mut cells: Vec<Cell<'_>> = Vec::with_capacity(columns.len());
                for (i, _column) in columns.iter().enumerate() {
                    if i == 0 {
                        let icon = if *collapsed { "▸" } else { "▾" };
                        let path_display = format_project_display_path(project, 52);
                        let count_label = if *count == 1 {
                            "(1 session)".to_owned()
                        } else {
                            format!("({count} sessions)")
                        };
                        let line = Line::from(vec![
                            Span::styled(
                                icon,
                                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                            ),
                            Span::raw(" "),
                            Span::styled(
                                path_display,
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::raw("  "),
                            Span::styled(count_label, Style::default().fg(MUTED)),
                        ]);
                        cells.push(Cell::from(line));
                    } else {
                        cells.push(Cell::from(""));
                    }
                }
                rows.push(Row::new(cells));
            }
            DisplayRow::Session { session_index } => {
                rows.push(session_row(*session_index));
            }
        }
    }

    let grouping_label = app.grouping.label();
    let table_title = format!(
        " Sessions  {} shown / {} saved  ·  grouped: {} ",
        app.filtered.len(),
        app.sessions.len(),
        grouping_label,
    );

    let table = Table::new(rows, columns.iter().map(|column| column.constraint))
        .header(header)
        .block(Block::new().borders(Borders::ALL).title(table_title))
        .column_spacing(1)
        .row_highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("› ");
    frame.render_stateful_widget(table, area, &mut app.table_state);
}

/// Stable label used as the grouping key for project grouping. Sessions without
/// a project land in a dedicated bucket so they still group together.
fn session_project_label(session: &AgentSession) -> String {
    session.project.as_deref().map_or_else(
        || "(no project)".to_owned(),
        |project| project.display().to_string(),
    )
}

fn render_session_detail_popup(frame: &mut Frame<'_>, app: &mut SessionsApp) {
    if app.mode != BrowserMode::Detail {
        return;
    }
    if app.detail.is_none() {
        return;
    }

    let popup = centered_rect(
        frame.area(),
        frame.area().width.saturating_sub(4).min(140),
        frame.area().height.saturating_sub(2),
    );
    let theme = app.detail_theme;
    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.border))
        .title_style(Style::default().fg(theme.popup_title))
        .title(" Session details ");
    let inner = block.inner(popup);
    let detail_status_height = app.detail_status.as_ref().map_or(0, |status| {
        let desired = wrapped_text_height(&Text::from(status.text.as_str()), inner.width.max(1));
        u16::try_from(desired)
            .unwrap_or(u16::MAX)
            .min(inner.height.saturating_sub(2))
    });
    let areas = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(detail_status_height),
        Constraint::Length(1),
    ])
    .split(inner);
    let content_width = areas[0].width.max(1);
    if app
        .detail_layout
        .as_ref()
        .is_none_or(|layout| layout.width != content_width)
    {
        let detail = app.detail.as_ref().expect("detail mode has detail data");
        app.detail_layout = Some(DetailLayoutCache::new(
            detail,
            content_width,
            theme,
            app.preview_scope,
        ));
    }
    let (content_height, primary_offsets) = {
        let layout = app
            .detail_layout
            .as_ref()
            .expect("detail layout was initialized");
        (layout.total_height(), layout.primary_offsets.clone())
    };
    let max_scroll = content_height.saturating_sub(usize::from(areas[0].height));
    app.detail_max_scroll = max_scroll;
    app.detail_scroll = app.detail_scroll.min(app.detail_max_scroll);
    app.detail_primary_offsets = primary_offsets
        .into_iter()
        .map(|offset| offset.min(app.detail_max_scroll))
        .collect();
    app.detail_primary_offsets.dedup();
    let (text, local_scroll) = app
        .detail_layout
        .as_ref()
        .expect("detail layout was initialized")
        .visible_text(app.detail_scroll, usize::from(areas[0].height));
    let paragraph = Paragraph::new(text).wrap(Wrap { trim: false });

    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    frame.render_widget(paragraph.scroll((local_scroll, 0)), areas[0]);
    render_detail_status_bar(frame, areas[1], app.detail_status.as_ref(), theme);
    render_detail_footer(frame, areas[2], theme);
}

fn render_detail_status_bar(
    frame: &mut Frame<'_>,
    area: Rect,
    status: Option<&StatusMessage>,
    theme: SessionDetailTheme,
) {
    let Some(status) = status else {
        return;
    };
    let color = if status.is_error {
        theme.status_error
    } else {
        theme.status_success
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            status.text.clone(),
            Style::default().fg(color),
        ))
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_detail_footer(frame: &mut Frame<'_>, area: Rect, theme: SessionDetailTheme) {
    frame.render_widget(
        Paragraph::new(themed_key_hints(
            &[
                ("Shift+↑/↓", "msg"),
                ("p", "chat"),
                ("Shift+P", "all"),
                ("c", "copy"),
                ("r", "resume"),
                ("e", "export"),
                ("Esc", "close"),
            ],
            theme.footer_key,
            theme.footer_text,
            theme.footer_separator,
        ))
        .alignment(Alignment::Center),
        area,
    );
}

struct SessionDetailContent {
    lines: Vec<Line<'static>>,
    primary_line_indices: Vec<usize>,
}

#[derive(Debug)]
struct DetailLayoutCache {
    width: u16,
    lines: Vec<Line<'static>>,
    line_offsets: Vec<usize>,
    primary_offsets: Vec<usize>,
}

impl DetailLayoutCache {
    fn new(
        detail: &SessionDetail,
        width: u16,
        theme: SessionDetailTheme,
        scope: DetailScope,
    ) -> Self {
        let width = width.max(1);
        let content = session_detail_content(detail, theme, scope);
        let mut lines = Vec::with_capacity(content.lines.len());
        let mut primary_line_indices = Vec::with_capacity(content.primary_line_indices.len());
        let mut primary_indices = content.primary_line_indices.into_iter().peekable();

        for (index, line) in content.lines.into_iter().enumerate() {
            while primary_indices
                .peek()
                .is_some_and(|primary| *primary == index)
            {
                primary_line_indices.push(lines.len());
                primary_indices.next();
            }
            lines.extend(fragment_detail_line(line));
        }

        let mut line_offsets = Vec::with_capacity(lines.len() + 1);
        line_offsets.push(0usize);
        for line in &lines {
            let height = Paragraph::new(Text::from(line.clone()))
                .wrap(Wrap { trim: false })
                .line_count(width)
                .max(1);
            let next = line_offsets
                .last()
                .copied()
                .unwrap_or_default()
                .saturating_add(height);
            line_offsets.push(next);
        }
        let primary_offsets = primary_line_indices
            .into_iter()
            .filter_map(|index| line_offsets.get(index).copied())
            .collect();

        Self {
            width,
            lines,
            line_offsets,
            primary_offsets,
        }
    }

    fn total_height(&self) -> usize {
        self.line_offsets.last().copied().unwrap_or_default()
    }

    fn visible_text(&self, scroll: usize, viewport_height: usize) -> (Text<'static>, u16) {
        if self.lines.is_empty() || scroll >= self.total_height() {
            return (Text::default(), 0);
        }

        let start_index = self
            .line_offsets
            .partition_point(|offset| *offset <= scroll)
            .saturating_sub(1)
            .min(self.lines.len() - 1);
        let local_scroll = scroll.saturating_sub(self.line_offsets[start_index]);
        let end_offset = scroll.saturating_add(viewport_height.max(1));
        let end_index = self
            .line_offsets
            .partition_point(|offset| *offset < end_offset)
            .max(start_index + 1)
            .min(self.lines.len());

        (
            Text::from(self.lines[start_index..end_index].to_vec()),
            u16::try_from(local_scroll).unwrap_or(u16::MAX),
        )
    }
}

fn fragment_detail_line(line: Line<'static>) -> Vec<Line<'static>> {
    // Paragraph::scroll accepts a u16 offset. Keeping each independently wrapped
    // logical line below that range lets the global scroll position remain usize.
    const MAX_CHARS_PER_LINE: usize = 8_192;

    if line
        .spans
        .iter()
        .map(|span| span.content.chars().count())
        .sum::<usize>()
        <= MAX_CHARS_PER_LINE
    {
        return vec![line];
    }

    let line_style = line.style;
    let alignment = line.alignment;
    let mut fragments = Vec::new();
    let mut spans = Vec::<Span<'static>>::new();
    let mut chars = 0usize;
    for span in line.spans {
        for character in span.content.chars() {
            if chars == MAX_CHARS_PER_LINE {
                fragments.push(Line {
                    style: line_style,
                    alignment,
                    spans: std::mem::take(&mut spans),
                });
                chars = 0;
            }
            if let Some(previous) = spans
                .last_mut()
                .filter(|previous| previous.style == span.style)
            {
                previous.content.to_mut().push(character);
            } else {
                spans.push(Span::styled(character.to_string(), span.style));
            }
            chars += 1;
        }
    }
    fragments.push(Line {
        style: line_style,
        alignment,
        spans,
    });
    fragments
}

fn session_detail_content(
    detail: &SessionDetail,
    theme: SessionDetailTheme,
    scope: DetailScope,
) -> SessionDetailContent {
    let mut lines = session_metadata_lines(detail, theme);
    lines.extend(model_usage_lines(detail, theme));
    let visible: Vec<&crate::session::SessionMessage> = detail.messages_in(scope).collect();
    let hidden = detail.hidden_message_count(scope);
    let header = if scope == DetailScope::Conversation {
        format!(
            "Conversation — {} shown (conversation only){}",
            visible.len(),
            if hidden > 0 {
                format!(", {hidden} tool/system hidden")
            } else {
                String::new()
            }
        )
    } else {
        format!(
            "Conversation — {} messages (complete)",
            detail.messages.len()
        )
    };
    lines.extend([
        Line::from(Span::styled(
            header,
            Style::default()
                .fg(theme.conversation_header)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ]);
    let mut primary_line_indices = Vec::new();
    for message in &visible {
        if matches!(
            message.kind,
            SessionMessageKind::User | SessionMessageKind::Assistant
        ) {
            primary_line_indices.push(lines.len());
        }
        lines.extend(session_message_lines(message, theme));
    }
    if detail.messages.is_empty() {
        lines.push(Line::from(Span::styled(
            "No persisted chat messages were found for this session.",
            Style::default().fg(theme.empty_text),
        )));
    } else if visible.is_empty() {
        lines.push(Line::from(Span::styled(
            "No user or assistant messages in this scope. Press Shift+P to show everything.",
            Style::default().fg(theme.empty_text),
        )));
    }
    SessionDetailContent {
        lines,
        primary_line_indices,
    }
}

fn session_metadata_lines(detail: &SessionDetail, theme: SessionDetailTheme) -> Vec<Line<'static>> {
    let session = &detail.session;
    vec![
        session_detail_line("Target", session.target(), theme),
        session_detail_line("Agent", session.kind.to_string(), theme),
        session_detail_line(
            "Title",
            session.title.as_deref().unwrap_or("(untitled)").to_owned(),
            theme,
        ),
        session_detail_line("Project", display_path(session.project.as_deref()), theme),
        session_detail_line(
            "Started",
            session.started_at.as_deref().unwrap_or("-").to_owned(),
            theme,
        ),
        session_detail_line(
            "Updated",
            format!(
                "{} ({})",
                format_unix_timestamp(session.updated_at),
                format_age(session.updated_at)
            ),
            theme,
        ),
        session_detail_line(
            "Tokens",
            session
                .tokens
                .map_or_else(|| "-".to_owned(), |value| value.to_string()),
            theme,
        ),
        session_detail_line("Cost", format_cost(session.cost_usd), theme),
        session_detail_line("Log file", session.path.display().to_string(), theme),
        Line::from(""),
    ]
}

fn model_usage_lines(detail: &SessionDetail, theme: SessionDetailTheme) -> Vec<Line<'static>> {
    let model_usage = detail.model_usage();
    if model_usage.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![Line::from(Span::styled(
        format!("Model usage ({} models)", model_usage.len()),
        Style::default()
            .fg(theme.conversation_header)
            .add_modifier(Modifier::BOLD),
    ))];
    for summary in &model_usage {
        lines.push(Line::from(Span::styled(
            format_model_usage_summary(summary),
            Style::default().fg(theme.metadata_value),
        )));
        if let Some(tokens) = format_token_breakdown(summary.tokens) {
            lines.push(Line::from(Span::styled(
                format!("Tokens: {tokens}"),
                Style::default().fg(theme.metadata_value),
            )));
        }
    }
    lines.push(Line::from(""));
    lines
}

fn session_message_lines(
    message: &crate::session::SessionMessage,
    theme: SessionDetailTheme,
) -> Vec<Line<'static>> {
    let timestamp = message.timestamp.as_deref().unwrap_or("-");
    let mut lines = vec![Line::from(Span::styled(
        message_header(message, timestamp),
        message_kind_style(message.kind, theme),
    ))];
    if let Some(response) = message.metrics.response.as_ref() {
        if let Some(tokens) = format_token_breakdown(response.tokens) {
            lines.push(Line::from(Span::styled(
                format!("Tokens: {tokens}"),
                message_body_style(message.kind, theme),
            )));
        }
        if let Some(summary) = format_response_summary(response) {
            lines.push(Line::from(Span::styled(
                format!("Response: {summary}"),
                message_body_style(message.kind, theme),
            )));
        }
        if let Some(error) = response.error.as_ref() {
            lines.push(Line::from(Span::styled(
                format!("Error: {}", format_metric_error(error)),
                message_body_style(SessionMessageKind::Error, theme),
            )));
        }
    }
    if matches!(
        message.kind,
        SessionMessageKind::Skill | SessionMessageKind::ToolCall
    ) {
        append_tool_lines(&mut lines, message, theme);
    }
    lines.extend(
        message
            .content
            .split('\n')
            .map(|line| line.strip_suffix('\r').unwrap_or(line))
            .map(|line| Line::styled(line.to_owned(), message_body_style(message.kind, theme))),
    );
    lines.push(Line::from(""));
    lines
}

fn append_tool_lines(
    lines: &mut Vec<Line<'static>>,
    message: &crate::session::SessionMessage,
    theme: SessionDetailTheme,
) {
    if let Some(tool) = message.metrics.tool.as_ref() {
        if let Some(summary) = format_tool_summary(tool) {
            lines.push(Line::from(Span::styled(
                format!("Tool: {summary}"),
                message_body_style(message.kind, theme),
            )));
        }
        if let Some(error) = tool.error.as_ref() {
            lines.push(Line::from(Span::styled(
                format!("Error: {}", format_metric_error(error)),
                message_body_style(SessionMessageKind::Error, theme),
            )));
        }
    }
    lines.push(Line::from(Span::styled(
        TOOL_TOKEN_ACCOUNTING_NOTE,
        message_body_style(message.kind, theme),
    )));
}

fn message_header(message: &crate::session::SessionMessage, timestamp: &str) -> String {
    let mut header = format!("[{timestamp}] {}", message.kind.label());
    if let Some(model) = message
        .model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        header.push_str(" · ");
        header.push_str(model);
    }
    if let Some(response) = message.metrics.response.as_ref() {
        for metric in format_response_header_metrics(response) {
            header.push_str(" · ");
            header.push_str(&metric);
        }
    }
    if matches!(
        message.kind,
        SessionMessageKind::Skill | SessionMessageKind::ToolCall
    ) && let Some(tool) = message.metrics.tool.as_ref()
        && let Some(summary) = format_tool_summary(tool)
    {
        header.push_str(" · ");
        header.push_str(&summary);
    }
    header
}

fn message_kind_style(kind: SessionMessageKind, theme: SessionDetailTheme) -> Style {
    Style::default()
        .fg(message_kind_color(kind, theme, true))
        .add_modifier(Modifier::BOLD)
}

fn message_body_style(kind: SessionMessageKind, theme: SessionDetailTheme) -> Style {
    Style::default().fg(message_kind_color(kind, theme, false))
}

const fn message_kind_color(
    kind: SessionMessageKind,
    theme: SessionDetailTheme,
    header: bool,
) -> Color {
    match (kind, header) {
        (SessionMessageKind::User, true) => theme.user_header,
        (SessionMessageKind::User, false) => theme.user_content,
        (SessionMessageKind::Assistant, true) => theme.assistant_header,
        (SessionMessageKind::Assistant, false) => theme.assistant_content,
        (SessionMessageKind::Skill, true) => theme.skill_header,
        (SessionMessageKind::Skill, false) => theme.skill_content,
        (SessionMessageKind::ToolCall, true) => theme.tool_call_header,
        (SessionMessageKind::ToolCall, false) => theme.tool_call_content,
        (SessionMessageKind::ToolResult, true) => theme.tool_result_header,
        (SessionMessageKind::ToolResult, false) => theme.tool_result_content,
        (SessionMessageKind::System, true) => theme.system_header,
        (SessionMessageKind::System, false) => theme.system_content,
        (SessionMessageKind::Error, true) => theme.error_header,
        (SessionMessageKind::Error, false) => theme.error_content,
    }
}

fn wrapped_text_height(text: &Text<'_>, width: u16) -> usize {
    Paragraph::new(text.clone())
        .wrap(Wrap { trim: false })
        .line_count(width.max(1))
}

fn render_session_footer(frame: &mut Frame<'_>, area: Rect, app: &SessionsApp) {
    let footer = app.search_in_progress.as_ref().map_or_else(
        || footer_for_status(app),
        |progress| searching_footer_line(progress, app),
    );
    frame.render_widget(Paragraph::new(footer).alignment(Alignment::Center), area);
}

fn footer_for_status(app: &SessionsApp) -> Line<'static> {
    app.status
        .as_ref()
        .map_or_else(session_key_hints, |status| {
            Line::from(Span::styled(status.text.clone(), status.style))
        })
}

fn searching_footer_line(progress: &InProgressSearch, app: &SessionsApp) -> Line<'static> {
    Line::from(Span::styled(
        search_progress_text(progress, app.sessions.len()),
        Style::default().fg(ACCENT),
    ))
}

/// Braille spinner frames; advanced by elapsed time so the animation ticks even
/// when each transcript loads faster than a frame.
const SEARCH_SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn search_progress_text(progress: &InProgressSearch, total: usize) -> String {
    let frame = SEARCH_SPINNER
        [progress.started.elapsed().as_millis() as usize / 100 % SEARCH_SPINNER.len()];
    format!(
        " {frame} Searching transcripts — {scanned}/{total} scanned, {hits} match{plural} (Esc cancel) ",
        scanned = progress.cursor.min(total),
        hits = progress.hits.len(),
        plural = if progress.hits.len() == 1 { "" } else { "es" },
    )
}

fn session_key_hints() -> Line<'static> {
    key_hints(&[
        ("↑/↓", "navigate"),
        ("/", "search"),
        ("g", "group"),
        ("Enter", "details"),
        ("r", "resume"),
        ("d", "delete"),
        ("q", "quit"),
    ])
}

fn render_delete_confirmation(frame: &mut Frame<'_>, app: &SessionsApp) {
    if app.mode != BrowserMode::ConfirmDelete {
        return;
    }
    let Some(session) = app.selected_session() else {
        return;
    };
    let popup = centered_rect(frame.area(), 72, 9);
    frame.render_widget(Clear, popup);
    let content = Text::from(vec![
        Line::from(Span::styled(
            "Permanently delete this session?",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(session.target()),
        Line::from(session.title.as_deref().unwrap_or("(untitled)")),
        Line::from(""),
        Line::from(Span::styled(
            "This removes native session files and known provider indexes. It cannot be undone.",
            Style::default().fg(Color::Yellow),
        )),
        Line::from(""),
        key_hints(&[("y", "delete permanently"), ("n/Esc", "cancel")]),
    ]);
    frame.render_widget(
        Paragraph::new(content)
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .block(
                Block::new()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Red))
                    .title(" Confirm deletion "),
            ),
        popup,
    );
}

#[derive(Debug, Clone, Copy)]
struct Column<T> {
    kind: T,
    label: &'static str,
    constraint: Constraint,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionColumn {
    Target,
    Active,
    Agent,
    Project,
    Title,
    Updated,
}

fn session_columns(width: u16) -> Vec<Column<SessionColumn>> {
    if width >= 120 {
        vec![
            column(SessionColumn::Target, "TARGET", Constraint::Length(44)),
            column(SessionColumn::Active, "", Constraint::Length(1)),
            column(SessionColumn::Agent, "AGENT", Constraint::Length(12)),
            column(SessionColumn::Project, "PROJECT", Constraint::Length(14)),
            column(SessionColumn::Title, "TITLE / SUMMARY", Constraint::Min(18)),
            column(SessionColumn::Updated, "UPDATED", Constraint::Length(11)),
        ]
    } else if width >= 80 {
        vec![
            column(SessionColumn::Target, "TARGET", Constraint::Length(44)),
            column(SessionColumn::Active, "", Constraint::Length(1)),
            column(SessionColumn::Title, "TITLE / SUMMARY", Constraint::Min(18)),
        ]
    } else {
        vec![
            column(SessionColumn::Target, "TARGET", Constraint::Length(44)),
            column(SessionColumn::Active, "", Constraint::Length(1)),
            column(SessionColumn::Title, "TITLE / SUMMARY", Constraint::Min(12)),
        ]
    }
}

fn session_cell(session: &AgentSession, column: SessionColumn, app: &SessionsApp) -> Cell<'static> {
    match column {
        SessionColumn::Active => {
            if app.active_targets.contains(&session.target()) {
                Cell::from(Span::styled("●", Style::default().fg(Color::Green)))
            } else {
                Cell::from("")
            }
        }
        SessionColumn::Agent => {
            let color = match session.kind {
                crate::process::AgentKind::ClaudeCode => Color::Yellow,
                crate::process::AgentKind::OhMyPi => Color::Cyan,
                crate::process::AgentKind::Pi => Color::LightMagenta,
                crate::process::AgentKind::Codex => Color::Green,
                crate::process::AgentKind::GeminiCli => Color::LightBlue,
                crate::process::AgentKind::OpenCode => Color::Blue,
                _ => Color::White,
            };
            Cell::from(Span::styled(
                session.kind.to_string(),
                Style::default().fg(color),
            ))
        }
        SessionColumn::Updated => {
            let age_str = format_age(session.updated_at);
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let age_secs = now.saturating_sub(session.updated_at);
            let style = if age_secs < 3600 {
                Style::default()
                    .fg(Color::LightGreen)
                    .add_modifier(Modifier::BOLD)
            } else if age_secs < 86400 {
                Style::default().fg(Color::Yellow)
            } else if age_secs < 7 * 86400 {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            Cell::from(Span::styled(age_str, style))
        }
        _ => Cell::from(session_value(session, column, app)),
    }
}
fn session_value(session: &AgentSession, column: SessionColumn, app: &SessionsApp) -> String {
    match column {
        SessionColumn::Target => session.target(),
        SessionColumn::Active => {
            if app.active_targets.contains(&session.target()) {
                "●".to_owned()
            } else {
                String::new()
            }
        }
        SessionColumn::Agent => session.kind.to_string(),
        SessionColumn::Project => project_label(session.project.as_deref()),
        SessionColumn::Title => session
            .title
            .clone()
            .unwrap_or_else(|| "(untitled)".to_owned()),
        SessionColumn::Updated => format_age(session.updated_at),
    }
}

const fn column<T>(kind: T, label: &'static str, constraint: Constraint) -> Column<T> {
    Column {
        kind,
        label,
        constraint,
    }
}

fn project_label(project: Option<&std::path::Path>) -> String {
    project.map_or_else(
        || "-".to_owned(),
        |project| {
            project.file_name().map_or_else(
                || project.display().to_string(),
                |name| name.to_string_lossy().into_owned(),
            )
        },
    )
}

fn display_path(project: Option<&std::path::Path>) -> String {
    project.map_or_else(|| "-".to_owned(), |project| project.display().to_string())
}

#[allow(dead_code)]
fn format_tokens_compact(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format_compact(tokens, 1_000_000, "M")
    } else if tokens >= 1_000 {
        format_compact(tokens, 1_000, "K")
    } else {
        tokens.to_string()
    }
}

#[allow(dead_code)]
fn format_compact(value: u64, unit: u64, suffix: &str) -> String {
    let mut whole = value / unit;
    let mut decimal = (value % unit * 10 + unit / 2) / unit;
    if decimal == 10 {
        whole += 1;
        decimal = 0;
    }
    format!("{whole}.{decimal}{suffix}")
}

fn format_cost(cost: Option<f64>) -> String {
    cost.map_or_else(|| "n/a".to_owned(), |cost| format!("${cost:.4}"))
}

fn format_age(updated_at: u64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(updated_at, |duration| duration.as_secs());
    format_duration(now.saturating_sub(updated_at))
}

fn format_unix_timestamp(timestamp: u64) -> String {
    i64::try_from(timestamp)
        .ok()
        .and_then(|timestamp| chrono::DateTime::from_timestamp(timestamp, 0))
        .map_or_else(|| timestamp.to_string(), |value| value.to_rfc3339())
}

#[allow(dead_code)]
fn status_style(status: &str) -> Style {
    match status {
        "running" => Style::default().fg(Color::Green),
        "stopped" | "exited" => Style::default().fg(Color::Red),
        "sleeping" | "idle" => Style::default().fg(Color::Gray),
        _ => Style::default(),
    }
}

#[allow(dead_code)]
fn detail_line(label: &'static str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<8}"), Style::default().fg(METADATA_KEY)),
        Span::raw(value),
    ])
}

fn session_detail_line(
    label: &'static str,
    value: String,
    theme: SessionDetailTheme,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{label:<8}"),
            Style::default().fg(theme.metadata_key),
        ),
        Span::styled(value, Style::default().fg(theme.metadata_value)),
    ])
}

fn key_hints(hints: &[(&str, &str)]) -> Line<'static> {
    themed_key_hints(hints, ACCENT, Color::Reset, MUTED)
}

fn themed_key_hints(
    hints: &[(&str, &str)],
    key_color: Color,
    text_color: Color,
    separator_color: Color,
) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, (key, action)) in hints.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled("  •  ", Style::default().fg(separator_color)));
        }
        spans.push(Span::styled(
            (*key).to_owned(),
            Style::default().fg(key_color).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            format!(" {action}"),
            Style::default().fg(text_color),
        ));
    }
    Line::from(spans)
}

fn centered_rect(area: Rect, preferred_width: u16, preferred_height: u16) -> Rect {
    let width = preferred_width.min(area.width.saturating_sub(2)).max(1);
    let height = preferred_height.min(area.height.saturating_sub(2)).max(1);
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

use std::collections::HashSet;

use crate::skill::{AgentSkill, SkillChildItem, SkillDetail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillFocus {
    List,
    Detail,
}

/// A flat "visual row" in the skill tree list.
#[derive(Debug, Clone)]
enum SkillRow {
    /// A top-level skill entry.
    Skill {
        skill_idx: usize,
        has_children: bool,
        expanded: bool,
    },
    /// Any item inside the skill directory tree (file or directory at any depth).
    Item {
        skill_idx: usize,
        full_path: PathBuf,
        name: String,
        is_dir: bool,
        expanded: bool,
        depth: usize,
        is_last: bool,
    },
}

/// Read a directory's children (dirs first, then files, each sorted).
fn read_dir_children(dir: &std::path::Path) -> Vec<SkillChildItem> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut items: Vec<SkillChildItem> = entries
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let name = p.file_name()?.to_str()?.to_string();
            let is_dir = p.is_dir();
            Some(SkillChildItem {
                name,
                path: p,
                is_dir,
            })
        })
        .collect();
    items.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    items
}

struct SkillsApp {
    skills: Vec<AgentSkill>,
    /// Set of expanded directory paths.
    expanded_dirs: HashSet<PathBuf>,
    /// Flat visible row list rebuilt by `rebuild_rows`.
    visible_rows: Vec<SkillRow>,
    selected_index: usize,
    search_query: String,
    is_searching: bool,
    current_detail: Option<SkillDetail>,
    /// Path of the file currently shown in the preview; used to detect stale cache.
    preview_path: Option<PathBuf>,
    preview_scroll: u16,
    full_screen_preview: bool,
    focus: SkillFocus,
    marquee_offset: usize,
    show_symlinks: bool,
}

impl SkillsApp {
    fn new(skills: Vec<AgentSkill>) -> Self {
        let mut app = Self {
            skills,
            expanded_dirs: HashSet::new(),
            visible_rows: Vec::new(),
            selected_index: 0,
            search_query: String::new(),
            is_searching: false,
            current_detail: None,
            preview_path: None,
            preview_scroll: 0,
            full_screen_preview: false,
            focus: SkillFocus::List,
            marquee_offset: 0,
            show_symlinks: false,
        };
        app.rebuild_rows();
        app
    }

    /// Rebuild `visible_rows` from skills, filter, and expansion state.
    fn rebuild_rows(&mut self) {
        let q = self.search_query.to_lowercase();
        self.visible_rows.clear();

        for (skill_idx, skill) in self.skills.iter().enumerate() {
            if !self.show_symlinks && skill.is_symlink {
                continue;
            }

            if !q.is_empty() {
                let hit = skill.name.to_lowercase().contains(&q)
                    || skill.provider.to_lowercase().contains(&q)
                    || skill.scope.to_lowercase().contains(&q)
                    || skill.location.to_lowercase().contains(&q)
                    || skill.triggers.iter().any(|t| t.to_lowercase().contains(&q))
                    || skill
                        .description
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&q);
                if !hit {
                    continue;
                }
            }

            let skill_dir = skill.path.parent().map(PathBuf::from);
            let has_children = skill_dir
                .as_ref()
                .is_some_and(|d| std::fs::read_dir(d).is_ok_and(|mut r| r.next().is_some()));
            let expanded = skill_dir
                .as_ref()
                .is_some_and(|d| self.expanded_dirs.contains(d));

            self.visible_rows.push(SkillRow::Skill {
                skill_idx,
                has_children,
                expanded,
            });

            if expanded && let Some(dir) = skill_dir {
                let children = read_dir_children(&dir);
                walk_children(
                    &mut self.visible_rows,
                    &self.expanded_dirs,
                    skill_idx,
                    &children,
                    1,
                );
            }
        }

        if self.selected_index >= self.visible_rows.len() {
            self.selected_index = self.visible_rows.len().saturating_sub(1);
        }
        self.preview_scroll = 0;
        self.marquee_offset = 0;
        self.current_detail = None;
        self.preview_path = None;
    }
}

/// Recursively walk children, adding visible rows for expanded directories.
fn walk_children(
    rows: &mut Vec<SkillRow>,
    expanded_dirs: &HashSet<PathBuf>,
    skill_idx: usize,
    children: &[SkillChildItem],
    depth: usize,
) {
    let total = children.len();
    for (i, child) in children.iter().enumerate() {
        let is_last = i + 1 == total;
        let expanded = child.is_dir && expanded_dirs.contains(&child.path);

        rows.push(SkillRow::Item {
            skill_idx,
            full_path: child.path.clone(),
            name: child.name.clone(),
            is_dir: child.is_dir,
            expanded,
            depth,
            is_last,
        });

        if expanded {
            let sub = read_dir_children(&child.path);
            walk_children(rows, expanded_dirs, skill_idx, &sub, depth + 1);
        }
    }
}

impl SkillsApp {
    /// Returns the filesystem path for the currently selected row.
    fn selected_preview_path(&self) -> Option<PathBuf> {
        let row = self.visible_rows.get(self.selected_index)?;
        match row {
            SkillRow::Skill { skill_idx, .. } => Some(self.skills[*skill_idx].path.clone()),
            SkillRow::Item { full_path, .. } => Some(full_path.clone()),
        }
    }

    /// The directory path that controls expansion for the current row (if any).
    fn selected_dir_path(&self) -> Option<PathBuf> {
        let row = self.visible_rows.get(self.selected_index)?;
        match row {
            SkillRow::Skill {
                skill_idx,
                has_children: true,
                ..
            } => self.skills[*skill_idx].path.parent().map(PathBuf::from),
            SkillRow::Item {
                full_path, is_dir, ..
            } if *is_dir => Some(full_path.clone()),
            _ => None,
        }
    }

    fn toggle_expand(&mut self) {
        let Some(dir_path) = self.selected_dir_path() else {
            return;
        };
        if self.expanded_dirs.contains(&dir_path) {
            self.expanded_dirs.remove(&dir_path);
        } else {
            self.expanded_dirs.insert(dir_path);
        }
        self.rebuild_rows();
    }

    fn collapse_current(&mut self) {
        let Some(row) = self.visible_rows.get(self.selected_index).cloned() else {
            return;
        };
        match row {
            SkillRow::Skill {
                skill_idx,
                expanded: true,
                ..
            } => {
                if let Some(dir) = self.skills[skill_idx].path.parent() {
                    self.expanded_dirs.remove(dir);
                    self.rebuild_rows();
                }
            }
            SkillRow::Item {
                full_path,
                is_dir: true,
                expanded: true,
                ..
            } => {
                self.expanded_dirs.remove(&full_path);
                self.rebuild_rows();
            }
            SkillRow::Item {
                skill_idx, depth, ..
            } => {
                // On a file/deeper item: collapse the nearest expanded ancestor dir
                // and move selection to it.
                let target_depth = depth.saturating_sub(1);
                self.collapse_ancestor(skill_idx, target_depth);
            }
            SkillRow::Skill { .. } => {}
        }
    }

    /// Find the expanded ancestor at `target_depth` for `skill_idx`,
    /// collapse it, and move selection to that row.
    fn collapse_ancestor(&mut self, skill_idx: usize, target_depth: usize) {
        // Scan visible_rows for the ancestor Item (same skill, same depth) or the Skill row.
        let mut found_idx: Option<usize> = None;
        let mut collapse_path: Option<PathBuf> = None;

        for (i, r) in self.visible_rows.iter().enumerate() {
            match r {
                SkillRow::Skill {
                    skill_idx: si,
                    expanded: true,
                    ..
                } if *si == skill_idx && target_depth == 0 => {
                    found_idx = Some(i);
                    collapse_path = self.skills[skill_idx].path.parent().map(PathBuf::from);
                    break;
                }
                SkillRow::Item {
                    skill_idx: si,
                    full_path,
                    depth: d,
                    expanded: true,
                    ..
                } if *si == skill_idx && *d == target_depth => {
                    found_idx = Some(i);
                    collapse_path = Some(full_path.clone());
                    break;
                }
                _ => {}
            }
        }

        if let Some(path) = collapse_path {
            self.expanded_dirs.remove(&path);
            self.rebuild_rows();
        }
        if let Some(i) = found_idx
            && i < self.visible_rows.len()
        {
            self.selected_index = i;
        }
    }

    const fn select_next(&mut self) {
        if !self.visible_rows.is_empty() {
            let max_idx = self.visible_rows.len() - 1;
            if self.selected_index < max_idx {
                self.selected_index += 1;
            }
        }
    }

    const fn select_prev(&mut self) {
        if !self.visible_rows.is_empty() && self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }
}

#[allow(clippy::too_many_lines)]
fn run_skill_browser(
    skills: Vec<AgentSkill>,
    load_detail: &mut impl FnMut(&AgentSkill) -> Result<SkillDetail>,
) -> Result<()> {
    let mut app = SkillsApp::new(skills);
    let mut terminal = ManagedTerminal::enter_with_native_selection()?;

    loop {
        // Refresh preview when selection changes (path-keyed cache)
        let current_path = app.selected_preview_path();
        if app.preview_path != current_path {
            app.preview_path.clone_from(&current_path);
            app.current_detail = None;
            app.preview_scroll = 0;
        }

        if app.current_detail.is_none()
            && let Some(_path) = &current_path
        {
            app.current_detail = match app.visible_rows.get(app.selected_index) {
                Some(SkillRow::Skill { skill_idx, .. }) => {
                    let skill = &app.skills[*skill_idx];
                    load_detail(skill).ok()
                }
                Some(SkillRow::Item {
                    full_path,
                    name,
                    is_dir,
                    skill_idx,
                    ..
                }) => {
                    let skill = &app.skills[*skill_idx];
                    let content = if *is_dir {
                        format!("[ directory: {name} ]")
                    } else {
                        std::fs::read_to_string(full_path)
                            .unwrap_or_else(|e| format!("(could not read: {e})"))
                    };
                    Some(SkillDetail {
                        skill: AgentSkill {
                            name: name.clone(),
                            provider: skill.provider.clone(),
                            scope: skill.scope.clone(),
                            path: full_path.clone(),
                            location: skill.location.clone(),
                            is_symlink: false,
                            description: None,
                            triggers: Vec::new(),
                            valid: true,
                            children: Vec::new(),
                        },
                        content,
                        extra: std::collections::BTreeMap::new(),
                    })
                }
                None => None,
            };
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
                            if let Some(path) = app.selected_preview_path() {
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

#[allow(clippy::too_many_lines)]
fn draw_skills(frame: &mut Frame, app: &SkillsApp) {
    let area = frame.area();

    let chunks = Layout::vertical([
        Constraint::Length(3), // Header
        Constraint::Min(10),   // Body
        Constraint::Length(1), // Footer
    ])
    .split(area);

    // 1. Header
    let header_text = if app.is_searching {
        format!(" MENA SKILLS  | Search: {}_ ", app.search_query)
    } else if !app.search_query.is_empty() {
        format!(
            " MENA SKILLS  | Filter: \"{}\" ({}/{})",
            app.search_query,
            app.visible_rows
                .iter()
                .filter(|r| matches!(r, SkillRow::Skill { .. }))
                .count(),
            app.skills.len()
        )
    } else {
        format!(
            " MENA SKILLS  | {} skills ",
            app.visible_rows
                .iter()
                .filter(|r| matches!(r, SkillRow::Skill { .. }))
                .count()
        )
    };

    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            " ⚡ ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            header_text,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan))
            .title(" Developer Agent Skills Browser "),
    );
    frame.render_widget(header, chunks[0]);

    // 2. Body
    if app.full_screen_preview {
        render_skill_preview(frame, chunks[1], app);
    } else {
        let body_chunks = Layout::horizontal([
            Constraint::Percentage(40), // List
            Constraint::Percentage(60), // Preview
        ])
        .split(chunks[1]);

        render_skill_list(frame, body_chunks[0], app);
        render_skill_preview(frame, body_chunks[1], app);
    }

    // 3. Footer
    let symlink_status = if app.show_symlinks {
        " (on) "
    } else {
        " (off) "
    };
    let footer_spans = vec![
        Span::styled(
            " Space/→",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Expand ", Style::default().fg(Color::DarkGray)),
        Span::styled("│", Style::default().fg(Color::DarkGray)),
        Span::styled(
            " ←",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Collapse ", Style::default().fg(Color::DarkGray)),
        Span::styled("│", Style::default().fg(Color::DarkGray)),
        Span::styled(
            " Tab/l",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Focus ", Style::default().fg(Color::DarkGray)),
        Span::styled("│", Style::default().fg(Color::DarkGray)),
        Span::styled(
            " ↑/↓",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Move ", Style::default().fg(Color::DarkGray)),
        Span::styled("│", Style::default().fg(Color::DarkGray)),
        Span::styled(
            " s",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" Symlinks{symlink_status}"),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled("│", Style::default().fg(Color::DarkGray)),
        Span::styled(
            " /",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Search ", Style::default().fg(Color::DarkGray)),
        Span::styled("│", Style::default().fg(Color::DarkGray)),
        Span::styled(
            " o",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Open ", Style::default().fg(Color::DarkGray)),
        Span::styled("│", Style::default().fg(Color::DarkGray)),
        Span::styled(
            " q/Esc",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Quit", Style::default().fg(Color::DarkGray)),
    ];
    let footer = Paragraph::new(Line::from(footer_spans));
    frame.render_widget(footer, chunks[2]);
}

#[allow(clippy::too_many_lines)]
fn render_skill_list(frame: &mut Frame, area: Rect, app: &SkillsApp) {
    let mut rows: Vec<Row> = Vec::new();

    // Pre-compute per-skill child counts for proper ├─ / └─ connectors
    let visible = &app.visible_rows;

    for (list_idx, row) in visible.iter().enumerate() {
        let is_selected = list_idx == app.selected_index;
        let row_bg = if is_selected {
            Style::default().bg(Color::DarkGray)
        } else {
            Style::default()
        };

        match row {
            SkillRow::Skill {
                skill_idx,
                has_children,
                expanded,
            } => {
                let skill = &app.skills[*skill_idx];

                let expand_icon = if *has_children {
                    if *expanded { "▾" } else { "▸" }
                } else {
                    " "
                };

                let cursor = if is_selected { "▶ " } else { "  " };
                let name_str = format!("{cursor}{expand_icon} {}", skill.name);
                let name_style = if is_selected {
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Reset)
                };

                let desc_raw = skill.description.as_deref().unwrap_or("-");
                let desc_display =
                    format_marquee_desc(desc_raw, 36, app.marquee_offset, is_selected);
                let desc_style = if is_selected {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Gray)
                };

                let (type_str, type_color) = if skill.is_symlink {
                    ("⇢ link", Color::Yellow)
                } else if *has_children {
                    ("dir", Color::LightBlue)
                } else {
                    ("md", Color::DarkGray)
                };

                rows.push(
                    Row::new(vec![
                        Cell::from(Span::styled(name_str, name_style)),
                        Cell::from(Span::styled(desc_display, desc_style)),
                        Cell::from(Span::styled(type_str, Style::default().fg(type_color))),
                    ])
                    .style(row_bg),
                );
            }
            SkillRow::Item {
                name,
                full_path,
                is_dir,
                expanded,
                depth,
                is_last,
                ..
            } => {
                // Build indentation + tree connector
                let indent: String = "   ".repeat(depth.saturating_sub(1));
                let connector = if *is_last { "└─ " } else { "├─ " };

                let expand_icon = if *is_dir {
                    if *expanded { "▾ " } else { "▸ " }
                } else {
                    ""
                };

                let name_str = format!("{indent}{connector}{expand_icon}{name}");

                let name_style = if is_selected {
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD)
                } else if *is_dir {
                    Style::default().fg(Color::LightBlue)
                } else {
                    Style::default().fg(Color::DarkGray)
                };

                let type_str: String = if *is_dir {
                    "dir".to_string()
                } else {
                    full_path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("file")
                        .to_string()
                };
                let type_color = if *is_dir {
                    Color::LightBlue
                } else {
                    Color::DarkGray
                };

                rows.push(
                    Row::new(vec![
                        Cell::from(Span::styled(name_str, name_style)),
                        Cell::from(""),
                        Cell::from(Span::styled(type_str, Style::default().fg(type_color))),
                    ])
                    .style(row_bg),
                );
            }
        }
    }

    let is_active_focus = app.focus == SkillFocus::List && !app.full_screen_preview;
    let (border_color, title_text) = if is_active_focus {
        (Color::Cyan, "▸ Skills Roster [ACTIVE] ")
    } else {
        (Color::DarkGray, " Skills Roster ")
    };

    let table = Table::new(
        rows,
        [
            Constraint::Min(20),   // NAME (with tree prefix)
            Constraint::Min(30),   // DESCRIPTION (wider)
            Constraint::Length(7), // TYPE (last, narrow)
        ],
    )
    .header(
        Row::new(vec!["NAME", "DESCRIPTION", "TYPE"]).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(title_text),
    );

    let mut state = TableState::default();
    if !app.visible_rows.is_empty() {
        state.select(Some(app.selected_index));
    }

    frame.render_stateful_widget(table, area, &mut state);
}

fn format_marquee_desc(desc: &str, max_len: usize, offset: usize, is_selected: bool) -> String {
    let clean = desc.split('\n').next().unwrap_or("-").trim();
    let char_count = clean.chars().count();
    if char_count <= max_len {
        return clean.to_string();
    }

    if !is_selected {
        let truncated: String = clean.chars().take(max_len.saturating_sub(3)).collect();
        return format!("{truncated}...");
    }

    let padded = format!("{clean}    ★    {clean}");
    let padded_chars: Vec<char> = padded.chars().collect();
    let cycle_len = char_count + 9;
    let start_pos = offset % cycle_len;

    if start_pos + max_len <= padded_chars.len() {
        padded_chars[start_pos..start_pos + max_len]
            .iter()
            .collect()
    } else {
        clean.chars().take(max_len).collect()
    }
}

#[allow(clippy::too_many_lines)]
fn render_skill_preview(frame: &mut Frame, area: Rect, app: &SkillsApp) {
    let is_active_focus = app.focus == SkillFocus::Detail && !app.full_screen_preview;
    let (border_color, title_text) = if app.full_screen_preview {
        (
            Color::Yellow,
            "▸ Skill Inspector & Details [FULLSCREEN] (Enter to exit) ",
        )
    } else if is_active_focus {
        (Color::Cyan, "▸ Skill Inspector & Details [ACTIVE] ")
    } else {
        (Color::DarkGray, " Skill Inspector & Details ")
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(title_text);

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    let Some(detail) = &app.current_detail else {
        let empty_p = Paragraph::new("No skill selected or failed to load content")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(empty_p, inner_area);
        return;
    };

    let skill = &detail.skill;

    let mut lines = vec![
        Line::from(vec![
            Span::styled("Name:        ", Style::default().fg(Color::LightMagenta)),
            Span::styled(
                &skill.name,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Provider:    ", Style::default().fg(Color::LightMagenta)),
            Span::styled(&skill.provider, Style::default().fg(Color::Yellow)),
            Span::raw("   "),
            Span::styled("Scope: ", Style::default().fg(Color::LightMagenta)),
            Span::styled(&skill.scope, Style::default().fg(Color::Cyan)),
            Span::raw("   "),
            Span::styled("Type: ", Style::default().fg(Color::LightMagenta)),
            Span::styled(
                if skill.is_symlink { "symlink" } else { "file" },
                Style::default().fg(if skill.is_symlink {
                    Color::Yellow
                } else {
                    Color::DarkGray
                }),
            ),
            Span::raw("   "),
            Span::styled("Valid: ", Style::default().fg(Color::LightMagenta)),
            Span::styled(
                if skill.valid { "✓ true" } else { "✗ false" },
                Style::default().fg(if skill.valid {
                    Color::Green
                } else {
                    Color::Red
                }),
            ),
        ]),
        Line::from(vec![
            Span::styled("Location:    ", Style::default().fg(Color::LightMagenta)),
            Span::styled(&skill.location, Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::styled("Path:        ", Style::default().fg(Color::LightMagenta)),
            Span::styled(
                skill.path.display().to_string(),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
    ];

    if !skill.triggers.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Triggers:    ", Style::default().fg(Color::LightMagenta)),
            Span::styled(
                skill.triggers.join(", "),
                Style::default().fg(Color::LightGreen),
            ),
        ]));
    }

    if let Some(desc) = &skill.description {
        lines.push(Line::from(vec![
            Span::styled("Description: ", Style::default().fg(Color::LightMagenta)),
            Span::styled(desc, Style::default().fg(Color::Gray)),
        ]));
    }

    lines.push(Line::from(Span::styled(
        "─────────────────────────────────────────────────────────────────────────────",
        Style::default().fg(Color::DarkGray),
    )));

    for content_line in detail.content.lines() {
        if content_line.starts_with("# ") {
            lines.push(Line::from(Span::styled(
                content_line,
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
        } else if content_line.starts_with("## ") || content_line.starts_with("### ") {
            lines.push(Line::from(Span::styled(
                content_line,
                Style::default()
                    .fg(Color::LightYellow)
                    .add_modifier(Modifier::BOLD),
            )));
        } else if content_line.starts_with("---") {
            lines.push(Line::from(Span::styled(
                content_line,
                Style::default().fg(Color::DarkGray),
            )));
        } else if content_line.starts_with("- ") || content_line.starts_with("* ") {
            lines.push(Line::from(vec![
                Span::styled("• ", Style::default().fg(Color::Cyan)),
                Span::raw(&content_line[2..]),
            ]));
        } else {
            lines.push(Line::from(content_line.to_string()));
        }
    }

    let total_lines = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    let visible_height = inner_area.height;
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll = app.preview_scroll.min(max_scroll);

    let paragraph = Paragraph::new(Text::from(lines))
        .scroll((scroll, 0))
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, inner_area);
}
#[cfg(test)]
mod tests {
    use super::DisplayRow;
    use std::collections::BTreeSet;
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

    use super::{
        BrowserMode, DetailAction, DetailScope, Grouping, InProgressSearch, SessionColumn,
        SessionDetailColorSettings, SessionDetailTheme, SessionsApp, StatusMessage,
        abort_message_search, coalesce_detail_events, draw_sessions, format_project_display_path,
        handle_detail_event, handle_detail_key, search_progress_text, session_cell,
        session_columns, session_project_label, start_message_search, step_message_search,
    };
    use crate::session::{
        AgentSession, DeletionSummary, ResponseMetrics, SessionDetail, SessionMessage,
        SessionMessageKind, SessionMessageMetrics, TokenUsage,
    };
    use crate::settings::ConfigColor;
    #[test]
    fn session_layout_displays_titles_and_filters_by_them() {
        let session = fixture_session();
        let mut app = SessionsApp::new(vec![session], BTreeSet::default());
        app.query = "rendering".to_owned();
        app.recompute_filter();
        assert_eq!(app.filtered.len(), 1);

        let mut terminal = Terminal::new(TestBackend::new(100, 18)).expect("test terminal");
        terminal
            .draw(|frame| draw_sessions(frame, &mut app))
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
            .draw(|frame| draw_sessions(frame, &mut app))
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
            .draw(|frame| draw_sessions(frame, &mut app))
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
            .draw(|frame| draw_sessions(frame, &mut app))
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
            .draw(|frame| draw_sessions(frame, &mut app))
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
            .draw(|frame| draw_sessions(frame, &mut app))
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
            .draw(|frame| draw_sessions(frame, &mut app))
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
            .draw(|frame| draw_sessions(frame, &mut app))
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
            .draw(|frame| draw_sessions(frame, &mut app))
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
            .draw(|frame| draw_sessions(frame, &mut app))
            .expect("draw details");

        handle_detail_key(
            &mut app,
            KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
            None,
            None,
        );
        terminal
            .draw(|frame| draw_sessions(frame, &mut app))
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
            .draw(|frame| draw_sessions(frame, &mut app))
            .expect("draw details");

        handle_detail_key(
            &mut app,
            KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
            None,
            None,
        );
        terminal
            .draw(|frame| draw_sessions(frame, &mut app))
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
            .draw(|frame| draw_sessions(frame, &mut app))
            .expect("draw wide details");
        let wide_max_scroll = app.detail_max_scroll;

        terminal.backend_mut().resize(36, 18);
        terminal
            .draw(|frame| draw_sessions(frame, &mut app))
            .expect("draw narrow details");
        assert!(app.detail_max_scroll > wide_max_scroll);
        handle_detail_key(
            &mut app,
            KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
            None,
            None,
        );
        terminal
            .draw(|frame| draw_sessions(frame, &mut app))
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
            .draw(|frame| draw_sessions(frame, &mut app))
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
            .draw(|frame| draw_sessions(frame, &mut app))
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
            .draw(|frame| draw_sessions(frame, &mut app))
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
        let mut export = |_detail: &SessionDetail, _scope: DetailScope| {
            Ok(PathBuf::from("/tmp/session-export.md"))
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
            .draw(|frame| draw_sessions(frame, &mut app))
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
            .draw(|frame| draw_sessions(frame, &mut app))
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
            children: Vec::new(),
        }
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
        let mut app = super::SkillsApp::new(vec![skill]);

        // Top-level collapsed: only the skill row.
        assert_eq!(app.visible_rows.len(), 1);
        assert!(matches!(app.visible_rows[0], super::SkillRow::Skill { .. }));

        // Expand the skill: references/ + SKILL.md appear (dirs first).
        app.selected_index = 0;
        app.toggle_expand();
        let rows = &app.visible_rows;
        assert_eq!(rows.len(), 3);
        assert!(matches!(
            rows[1],
            super::SkillRow::Item {
                is_dir: true,
                depth: 1,
                ..
            }
        ));

        // Select references dir and expand: examples/ + guide.md appear (dirs first).
        app.selected_index = 1;
        app.toggle_expand();
        let rows = &app.visible_rows;
        assert!(rows.iter().any(|r| matches!(
            r,
            super::SkillRow::Item { depth: 2, is_dir: true, name, .. } if name == "examples"
        )));
        assert!(rows.iter().any(|r| matches!(
            r,
            super::SkillRow::Item { depth: 2, is_dir: false, name, .. } if name == "guide.md"
        )));

        // Expand examples: demo.md at depth 3 appears.
        let examples_idx = rows
            .iter()
            .position(|r| matches!(r, super::SkillRow::Item { name, .. } if name == "examples"))
            .expect("examples row");
        app.selected_index = examples_idx;
        app.toggle_expand();
        let rows = &app.visible_rows;
        assert!(rows.iter().any(|r| matches!(
            r,
            super::SkillRow::Item { depth: 3, is_dir: false, name, .. } if name == "demo.md"
        )));

        // Collapse references: its subtree disappears.
        let references_idx = rows
            .iter()
            .position(|r| matches!(r, super::SkillRow::Item { name, .. } if name == "references"))
            .expect("references row");
        app.selected_index = references_idx;
        app.collapse_current();
        let rows = &app.visible_rows;
        assert_eq!(rows.len(), 3);
        assert!(
            !rows
                .iter()
                .any(|r| matches!(r, super::SkillRow::Item { name, .. } if name == "demo.md"))
        );

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
