use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseEventKind,
};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, TableState, Wrap};
use ratatui::{DefaultTerminal, Frame};

use crate::session::{AgentSession, DeletionSummary, SessionDetail, SessionMessageKind};
use crate::view::{AgentReport, format_bytes, format_duration};

const ACCENT: Color = Color::Cyan;
const MUTED: Color = Color::DarkGray;
const METADATA_KEY: Color = Color::LightMagenta;
type DeleteCallback<'a> = &'a mut dyn FnMut(&AgentSession) -> Result<DeletionSummary>;
type DetailCallback<'a> = &'a mut dyn FnMut(&AgentSession) -> Result<SessionDetail>;
type ExportCallback<'a> = &'a mut dyn FnMut(&SessionDetail) -> Result<PathBuf>;
type CopyCallback<'a> = &'a mut dyn FnMut(&SessionDetail) -> Result<()>;

pub fn run_top(
    interval: Duration,
    mut refresh: impl FnMut() -> Result<Vec<AgentReport>>,
) -> Result<()> {
    let reports = refresh()?;
    let mut app = TopApp::new(reports);
    let mut terminal = ManagedTerminal::enter()?;
    let mut deadline = Instant::now() + interval;

    loop {
        terminal
            .terminal
            .draw(|frame| draw_top(frame, &mut app))
            .context("failed to draw interactive agent view")?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero()
            || !event::poll(remaining).context("failed to poll terminal input")?
        {
            app.replace(refresh()?);
            deadline = Instant::now() + interval;
            continue;
        }
        match event::read().context("failed to read terminal input")? {
            Event::Key(key) if is_key_press(&key) => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(());
                }
                KeyCode::Up | KeyCode::Char('k') => app.previous(),
                KeyCode::Down | KeyCode::Char('j') => app.next(),
                KeyCode::Home => app.first(),
                KeyCode::End => app.last(),
                KeyCode::Enter | KeyCode::Char('i') => app.show_details = !app.show_details,
                KeyCode::Char('r') => {
                    app.replace(refresh()?);
                    deadline = Instant::now() + interval;
                }
                _ => {}
            },
            _ => {}
        }
    }
}

pub fn manage_sessions(
    sessions: Vec<AgentSession>,
    active_targets: BTreeSet<String>,
    mut load_detail: impl FnMut(&AgentSession) -> Result<SessionDetail>,
    mut export: impl FnMut(&SessionDetail) -> Result<PathBuf>,
    mut copy: impl FnMut(&SessionDetail) -> Result<()>,
    mut delete: impl FnMut(&AgentSession) -> Result<DeletionSummary>,
) -> Result<Option<AgentSession>> {
    run_session_browser(
        sessions,
        active_targets,
        BrowserPurpose::Manage,
        Some(&mut load_detail),
        Some(&mut export),
        Some(&mut copy),
        Some(&mut delete),
    )
}

pub fn pick_session(sessions: Vec<AgentSession>) -> Result<Option<AgentSession>> {
    run_session_browser(
        sessions,
        BTreeSet::new(),
        BrowserPurpose::Pick,
        None,
        None,
        None,
        None,
    )
}

fn run_session_browser(
    sessions: Vec<AgentSession>,
    active_targets: BTreeSet<String>,
    purpose: BrowserPurpose,
    mut load_detail: Option<DetailCallback<'_>>,
    mut export: Option<ExportCallback<'_>>,
    mut copy: Option<CopyCallback<'_>>,
    mut delete: Option<DeleteCallback<'_>>,
) -> Result<Option<AgentSession>> {
    let mut app = SessionsApp::new(sessions, active_targets, purpose);
    let mut terminal = ManagedTerminal::enter_with_mouse_capture()?;
    loop {
        terminal
            .terminal
            .draw(|frame| draw_sessions(frame, &mut app))
            .context("failed to draw session browser")?;
        let input = event::read().context("failed to read terminal input")?;
        if app.mode == BrowserMode::Detail {
            for input in read_detail_event_batch(input)? {
                let action = handle_detail_browser_event(&mut app, &input, &mut export, &mut copy);
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
                    handle_search_key(&mut app, key);
                    continue;
                }
                if app.mode == BrowserMode::ConfirmDelete {
                    match key.code {
                        KeyCode::Char('y') => {
                            if let Some(session) = app.selected_session().cloned()
                                && let Some(delete) = delete.as_deref_mut()
                            {
                                match delete(&session) {
                                    Ok(summary) => app.deleted(&session, summary),
                                    Err(error) => {
                                        app.status = Some(StatusMessage::error(format!(
                                            "Delete failed: {error:#}"
                                        )));
                                        app.mode = BrowserMode::Browse;
                                    }
                                }
                            }
                        }
                        KeyCode::Char('n') | KeyCode::Esc => {
                            app.mode = BrowserMode::Browse;
                        }
                        _ => {}
                    }
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
                    KeyCode::Enter if app.purpose == BrowserPurpose::Pick => {
                        return Ok(app.selected_session().cloned());
                    }
                    KeyCode::Char('r') if app.purpose == BrowserPurpose::Manage => {
                        return Ok(app.selected_session().cloned());
                    }
                    KeyCode::Enter | KeyCode::Char('i')
                        if app.purpose == BrowserPurpose::Manage =>
                    {
                        if let Some(session) = app.selected_session().cloned()
                            && let Some(load_detail) = load_detail.as_deref_mut()
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
                    KeyCode::Char('d') if app.purpose == BrowserPurpose::Manage => {
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
            app.recompute_filter();
            app.mode = BrowserMode::Browse;
        }
        KeyCode::Enter => app.mode = BrowserMode::Browse,
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
}

struct ManagedTerminal {
    terminal: DefaultTerminal,
    mouse_capture: bool,
}

impl ManagedTerminal {
    fn enter() -> Result<Self> {
        Self::enter_internal(false)
    }

    fn enter_with_mouse_capture() -> Result<Self> {
        Self::enter_internal(true)
    }

    fn enter_internal(mouse_capture: bool) -> Result<Self> {
        let mut terminal = ratatui::try_init().context("failed to initialize terminal UI")?;
        if mouse_capture
            && let Err(error) = crossterm::execute!(std::io::stdout(), EnableMouseCapture)
        {
            let _ = ratatui::try_restore();
            return Err(error).context("failed to enable terminal mouse capture");
        }
        if let Err(error) = terminal.hide_cursor() {
            if mouse_capture {
                let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
            }
            let _ = ratatui::try_restore();
            return Err(error).context("failed to hide terminal cursor");
        }
        Ok(Self {
            terminal,
            mouse_capture,
        })
    }
}

impl Drop for ManagedTerminal {
    fn drop(&mut self) {
        let _ = self.terminal.show_cursor();
        if self.mouse_capture {
            let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
        }
        let _ = ratatui::try_restore();
    }
}

#[derive(Debug)]
struct TopApp {
    reports: Vec<AgentReport>,
    table_state: TableState,
    show_details: bool,
}

impl TopApp {
    fn new(reports: Vec<AgentReport>) -> Self {
        let mut app = Self {
            reports,
            table_state: TableState::default(),
            show_details: false,
        };
        app.first();
        app
    }

    fn replace(&mut self, reports: Vec<AgentReport>) {
        let selected = self.table_state.selected().unwrap_or_default();
        self.reports = reports;
        self.table_state.select(
            self.reports
                .len()
                .checked_sub(1)
                .map(|last| selected.min(last)),
        );
    }

    fn previous(&mut self) {
        let selected = self.table_state.selected().unwrap_or_default();
        self.table_state.select(Some(selected.saturating_sub(1)));
    }

    fn next(&mut self) {
        let selected = self.table_state.selected().unwrap_or_default();
        if selected + 1 < self.reports.len() {
            self.table_state.select(Some(selected + 1));
        }
    }

    fn first(&mut self) {
        self.table_state
            .select((!self.reports.is_empty()).then_some(0));
    }

    const fn last(&mut self) {
        self.table_state.select(self.reports.len().checked_sub(1));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrowserPurpose {
    Manage,
    Pick,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrowserMode {
    Browse,
    Search,
    Detail,
    ConfirmDelete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DetailAction {
    Continue,
    Resume,
}

#[derive(Debug)]
struct SessionsApp {
    sessions: Vec<AgentSession>,
    filtered: Vec<usize>,
    active_targets: BTreeSet<String>,
    table_state: TableState,
    query: String,
    mode: BrowserMode,
    purpose: BrowserPurpose,
    detail: Option<SessionDetail>,
    detail_scroll: usize,
    detail_max_scroll: usize,
    detail_primary_offsets: Vec<usize>,
    detail_layout: Option<DetailLayoutCache>,
    detail_status: Option<StatusMessage>,
    status: Option<StatusMessage>,
}

impl SessionsApp {
    fn new(
        sessions: Vec<AgentSession>,
        active_targets: BTreeSet<String>,
        purpose: BrowserPurpose,
    ) -> Self {
        let mut app = Self {
            sessions,
            filtered: Vec::new(),
            active_targets,
            table_state: TableState::default(),
            query: String::new(),
            mode: BrowserMode::Browse,
            purpose,
            detail: None,
            detail_scroll: 0,
            detail_max_scroll: 0,
            detail_primary_offsets: Vec::new(),
            detail_layout: None,
            detail_status: None,
            status: None,
        };
        app.recompute_filter();
        app
    }

    fn recompute_filter(&mut self) {
        let query = self.query.to_ascii_lowercase();
        self.filtered = self
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, session)| session_matches(session, &query))
            .map(|(index, _)| index)
            .collect();
        let selected = self.table_state.selected().unwrap_or_default();
        self.table_state.select(
            self.filtered
                .len()
                .checked_sub(1)
                .map(|last| selected.min(last)),
        );
    }

    fn append_search(&mut self, value: &str) {
        self.query.push_str(value);
        self.recompute_filter();
    }

    fn selected_session(&self) -> Option<&AgentSession> {
        self.table_state
            .selected()
            .and_then(|selected| self.filtered.get(selected))
            .and_then(|index| self.sessions.get(*index))
    }

    fn previous(&mut self) {
        self.move_by(-1);
    }

    fn next(&mut self) {
        self.move_by(1);
    }

    fn move_by(&mut self, amount: isize) {
        if self.filtered.is_empty() {
            return;
        }
        let selected = self.table_state.selected().unwrap_or_default();
        let selected = selected
            .saturating_add_signed(amount)
            .min(self.filtered.len() - 1);
        self.table_state.select(Some(selected));
    }

    fn first(&mut self) {
        self.table_state
            .select((!self.filtered.is_empty()).then_some(0));
    }

    const fn last(&mut self) {
        self.table_state.select(self.filtered.len().checked_sub(1));
    }

    fn open_detail(&mut self, detail: SessionDetail) {
        self.detail = Some(detail);
        self.detail_scroll = 0;
        self.detail_max_scroll = 0;
        self.detail_primary_offsets.clear();
        self.detail_layout = None;
        self.detail_status = None;
        self.mode = BrowserMode::Detail;
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
        if self.active_targets.contains(&session_target(session)) {
            self.status = Some(StatusMessage::error(
                "Cannot delete a session that is attached to a running agent".to_owned(),
            ));
        } else {
            self.mode = BrowserMode::ConfirmDelete;
        }
    }

    fn deleted(&mut self, deleted: &AgentSession, summary: DeletionSummary) {
        let target = session_target(deleted);
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
    match key.code {
        KeyCode::Esc | KeyCode::Enter | KeyCode::Char('i' | 'q') => app.close_detail(),
        KeyCode::Char('c') => {
            let result = app
                .detail
                .as_ref()
                .zip(copy)
                .map(|(detail, copy)| copy(detail));
            if let Some(result) = result {
                app.detail_status = Some(match result {
                    Ok(()) => {
                        StatusMessage::success("Copied complete detail to clipboard".to_owned())
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
                .map(|(detail, export)| export(detail));
            if let Some(result) = result {
                app.detail_status = Some(match result {
                    Ok(path) => StatusMessage::success(format!("Exported: {}", path.display())),
                    Err(error) => StatusMessage::error(format!("Export failed: {error:#}")),
                });
            }
        }
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
        _ => None,
    }
}

#[derive(Debug)]
struct StatusMessage {
    text: String,
    style: Style,
}

impl StatusMessage {
    fn success(text: String) -> Self {
        Self {
            text,
            style: Style::default().fg(Color::Green),
        }
    }

    fn error(text: String) -> Self {
        Self {
            text,
            style: Style::default().fg(Color::Red),
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

fn draw_top(frame: &mut Frame<'_>, app: &mut TopApp) {
    let details_height = if app.show_details && frame.area().height >= 15 {
        7
    } else {
        0
    };
    let areas = Layout::vertical([
        Constraint::Min(5),
        Constraint::Length(details_height),
        Constraint::Length(1),
    ])
    .split(frame.area());

    let columns = top_columns(areas[0].width);
    let header = Row::new(columns.iter().map(|column| Cell::from(column.label)))
        .style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .bottom_margin(1);
    let rows = app.reports.iter().map(|report| {
        Row::new(
            columns
                .iter()
                .map(|column| Cell::from(top_value(report, column.kind))),
        )
        .style(status_style(&report.agent.process.status))
    });
    let widths = columns.iter().map(|column| column.constraint);
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::new().borders(Borders::ALL).title(Line::from(vec![
            Span::styled(" mena top ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(
                format!(" {} running ", app.reports.len()),
                Style::default().fg(ACCENT),
            ),
        ])))
        .column_spacing(1)
        .row_highlight_style(
            Style::default()
                .bg(Color::DarkGray)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("› ");
    frame.render_stateful_widget(table, areas[0], &mut app.table_state);

    if details_height > 0 {
        render_top_details(frame, areas[1], selected_report(app));
    }

    render_top_footer(frame, areas[2]);
}

fn render_top_details(frame: &mut Frame<'_>, area: Rect, report: Option<&AgentReport>) {
    let Some(report) = report else {
        return;
    };
    let session = report
        .session
        .as_ref()
        .map_or_else(|| "-".to_owned(), session_target);
    let details = Text::from(vec![
        detail_line(
            "Process",
            format!(
                "{}:{}  PID {}  {}",
                report.agent.kind,
                report.agent.process.pid,
                report.agent.process.pid,
                report.agent.process.status
            ),
        ),
        detail_line("Project", display_path(report.project())),
        detail_line("Session", session),
        detail_line(
            "Usage",
            format!(
                "{} tokens  •  {}",
                report
                    .session
                    .as_ref()
                    .and_then(|session| session.tokens)
                    .map_or_else(|| "-".to_owned(), |tokens| tokens.to_string()),
                report.session.as_ref().map_or_else(
                    || "cost -".to_owned(),
                    |session| format!("cost {}", format_cost(session.cost_usd))
                )
            ),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(details)
            .block(Block::new().borders(Borders::ALL).title(" Details "))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_top_footer(frame: &mut Frame<'_>, area: Rect) {
    frame.render_widget(
        Paragraph::new(key_hints(&[
            ("↑/↓", "navigate"),
            ("Enter", "details"),
            ("r", "refresh"),
            ("q", "quit"),
        ]))
        .alignment(Alignment::Center),
        area,
    );
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

fn render_session_table_widget(frame: &mut Frame<'_>, area: Rect, app: &mut SessionsApp) {
    let columns = session_columns(area.width);
    let header = Row::new(columns.iter().map(|column| Cell::from(column.label)))
        .style(
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .bottom_margin(1);
    let rows = app.filtered.iter().filter_map(|index| {
        let session = app.sessions.get(*index)?;
        Some(Row::new(columns.iter().map(|column| {
            Cell::from(session_value(session, column.kind, app))
        })))
    });
    let table_title = match app.purpose {
        BrowserPurpose::Manage => format!(
            " Sessions  {} shown / {} saved ",
            app.filtered.len(),
            app.sessions.len()
        ),
        BrowserPurpose::Pick => format!(" Resume session  {} matches ", app.filtered.len()),
    };
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
    let block = Block::new()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT))
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
        app.detail_layout = Some(DetailLayoutCache::new(detail, content_width));
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
    if let Some(status) = app.detail_status.as_ref() {
        frame.render_widget(
            Paragraph::new(Span::styled(status.text.clone(), status.style))
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: false }),
            areas[1],
        );
    }
    frame.render_widget(
        Paragraph::new(key_hints(&[
            ("Shift+↑/↓", "message"),
            ("c", "copy"),
            ("r", "resume"),
            ("e", "export"),
            ("Enter/Esc", "close"),
        ]))
        .alignment(Alignment::Center),
        areas[2],
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
    fn new(detail: &SessionDetail, width: u16) -> Self {
        let width = width.max(1);
        let content = session_detail_content(detail);
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

fn session_detail_content(detail: &SessionDetail) -> SessionDetailContent {
    let session = &detail.session;
    let mut lines = vec![
        detail_line("Target", session_target(session)),
        detail_line("Agent", session.kind.to_string()),
        detail_line(
            "Title",
            session.title.as_deref().unwrap_or("(untitled)").to_owned(),
        ),
        detail_line("Project", display_path(session.project.as_deref())),
        detail_line(
            "Started",
            session.started_at.as_deref().unwrap_or("-").to_owned(),
        ),
        detail_line(
            "Updated",
            format!(
                "{} ({})",
                format_unix_timestamp(session.updated_at),
                format_age(session.updated_at)
            ),
        ),
        detail_line(
            "Tokens",
            session
                .tokens
                .map_or_else(|| "-".to_owned(), |value| value.to_string()),
        ),
        detail_line("Cost", format_cost(session.cost_usd)),
        detail_line("Log file", session.path.display().to_string()),
        Line::from(""),
        Line::from(Span::styled(
            format!("Conversation ({} messages)", detail.messages.len()),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    let mut primary_line_indices = Vec::new();
    for message in &detail.messages {
        if matches!(
            message.kind,
            SessionMessageKind::User | SessionMessageKind::Assistant
        ) {
            primary_line_indices.push(lines.len());
        }
        let timestamp = message.timestamp.as_deref().unwrap_or("-");
        lines.push(Line::from(Span::styled(
            message_header(message, timestamp),
            message_kind_style(message.kind),
        )));
        lines.extend(
            message
                .content
                .split('\n')
                .map(|line| line.strip_suffix('\r').unwrap_or(line))
                .map(|line| Line::styled(line.to_owned(), message_body_style(message.kind))),
        );
        lines.push(Line::from(""));
    }
    if detail.messages.is_empty() {
        lines.push(Line::from(Span::styled(
            "No persisted chat messages were found for this session.",
            Style::default().fg(MUTED),
        )));
    }
    SessionDetailContent {
        lines,
        primary_line_indices,
    }
}

fn message_header(message: &crate::session::SessionMessage, timestamp: &str) -> String {
    let model = (message.kind == SessionMessageKind::Assistant)
        .then(|| message.model.as_deref().map(str::trim))
        .flatten()
        .filter(|model| !model.is_empty());
    model.map_or_else(
        || format!("[{timestamp}] {}", message.kind.label()),
        |model| format!("[{timestamp}] {} · {model}", message.kind.label()),
    )
}

fn message_kind_style(kind: SessionMessageKind) -> Style {
    Style::default()
        .fg(message_kind_color(kind))
        .add_modifier(Modifier::BOLD)
}

fn message_body_style(kind: SessionMessageKind) -> Style {
    Style::default().fg(message_kind_color(kind))
}

const fn message_kind_color(kind: SessionMessageKind) -> Color {
    match kind {
        SessionMessageKind::User => Color::LightGreen,
        SessionMessageKind::Assistant => Color::Cyan,
        SessionMessageKind::Skill => Color::LightYellow,
        SessionMessageKind::ToolCall
        | SessionMessageKind::ToolResult
        | SessionMessageKind::System
        | SessionMessageKind::Error => Color::DarkGray,
    }
}

fn wrapped_text_height(text: &Text<'_>, width: u16) -> usize {
    Paragraph::new(text.clone())
        .wrap(Wrap { trim: false })
        .line_count(width.max(1))
}

fn render_session_footer(frame: &mut Frame<'_>, area: Rect, app: &SessionsApp) {
    let footer = app.status.as_ref().map_or_else(
        || session_key_hints(app.purpose),
        |status| Line::from(Span::styled(status.text.clone(), status.style)),
    );
    frame.render_widget(Paragraph::new(footer).alignment(Alignment::Center), area);
}

fn session_key_hints(purpose: BrowserPurpose) -> Line<'static> {
    if purpose == BrowserPurpose::Pick {
        key_hints(&[
            ("↑/↓", "navigate"),
            ("/", "search"),
            ("Enter", "resume"),
            ("q", "cancel"),
        ])
    } else {
        key_hints(&[
            ("↑/↓", "navigate"),
            ("/", "search"),
            ("Enter", "details"),
            ("r", "resume"),
            ("d", "delete"),
            ("q", "quit"),
        ])
    }
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
        Line::from(session_target(session)),
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

#[derive(Debug, Clone, Copy)]
enum TopColumn {
    Id,
    Agent,
    Project,
    Status,
    Duration,
    Cpu,
    Memory,
    Tokens,
    Cost,
}

fn top_columns(width: u16) -> Vec<Column<TopColumn>> {
    if width >= 120 {
        vec![
            column(TopColumn::Id, "ID", Constraint::Length(14)),
            column(TopColumn::Agent, "AGENT", Constraint::Length(12)),
            column(TopColumn::Project, "PROJECT", Constraint::Min(16)),
            column(TopColumn::Status, "STATUS", Constraint::Length(9)),
            column(TopColumn::Duration, "DURATION", Constraint::Length(9)),
            column(TopColumn::Cpu, "CPU", Constraint::Length(6)),
            column(TopColumn::Memory, "MEMORY", Constraint::Length(10)),
            column(TopColumn::Tokens, "TOKENS", Constraint::Length(9)),
            column(TopColumn::Cost, "COST", Constraint::Length(9)),
        ]
    } else if width >= 88 {
        vec![
            column(TopColumn::Id, "ID", Constraint::Length(14)),
            column(TopColumn::Agent, "AGENT", Constraint::Length(11)),
            column(TopColumn::Project, "PROJECT", Constraint::Min(13)),
            column(TopColumn::Status, "STATUS", Constraint::Length(9)),
            column(TopColumn::Cpu, "CPU", Constraint::Length(6)),
            column(TopColumn::Memory, "MEMORY", Constraint::Length(10)),
            column(TopColumn::Tokens, "TOKENS", Constraint::Length(9)),
            column(TopColumn::Cost, "COST", Constraint::Length(8)),
        ]
    } else if width >= 60 {
        vec![
            column(TopColumn::Id, "ID", Constraint::Length(14)),
            column(TopColumn::Project, "PROJECT", Constraint::Min(12)),
            column(TopColumn::Status, "STATUS", Constraint::Length(9)),
            column(TopColumn::Cpu, "CPU", Constraint::Length(6)),
            column(TopColumn::Tokens, "TOKENS", Constraint::Length(9)),
        ]
    } else {
        vec![
            column(TopColumn::Id, "ID", Constraint::Length(14)),
            column(TopColumn::Project, "PROJECT", Constraint::Min(8)),
            column(TopColumn::Status, "STATUS", Constraint::Length(9)),
        ]
    }
}

fn top_value(report: &AgentReport, column: TopColumn) -> String {
    match column {
        TopColumn::Id => format!("{}:{}", report.agent.kind.slug(), report.agent.process.pid),
        TopColumn::Agent => report.agent.kind.to_string(),
        TopColumn::Project => project_label(report.project()),
        TopColumn::Status => report.agent.process.status.clone(),
        TopColumn::Duration => format_duration(report.agent.process.run_time),
        TopColumn::Cpu => format!("{:.1}%", report.agent.process.cpu_percent),
        TopColumn::Memory => format_bytes(report.agent.process.memory_bytes),
        TopColumn::Tokens => report
            .session
            .as_ref()
            .and_then(|session| session.tokens)
            .map_or_else(|| "-".to_owned(), format_tokens_compact),
        TopColumn::Cost => report
            .session
            .as_ref()
            .map_or_else(|| "-".to_owned(), |session| format_cost(session.cost_usd)),
    }
}

#[derive(Debug, Clone, Copy)]
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
            column(SessionColumn::Target, "TARGET", Constraint::Length(46)),
            column(SessionColumn::Active, "", Constraint::Length(1)),
            column(SessionColumn::Agent, "AGENT", Constraint::Length(11)),
            column(SessionColumn::Project, "PROJECT", Constraint::Length(14)),
            column(SessionColumn::Title, "TITLE / SUMMARY", Constraint::Min(18)),
            column(SessionColumn::Updated, "UPDATED", Constraint::Length(11)),
        ]
    } else if width >= 80 {
        vec![
            column(SessionColumn::Target, "TARGET", Constraint::Length(46)),
            column(SessionColumn::Active, "", Constraint::Length(1)),
            column(SessionColumn::Title, "TITLE / SUMMARY", Constraint::Min(18)),
        ]
    } else {
        vec![
            column(SessionColumn::Target, "TARGET", Constraint::Length(46)),
            column(SessionColumn::Active, "", Constraint::Length(1)),
            column(SessionColumn::Title, "TITLE / SUMMARY", Constraint::Min(12)),
        ]
    }
}

fn session_value(session: &AgentSession, column: SessionColumn, app: &SessionsApp) -> String {
    match column {
        SessionColumn::Target => session_target(session),
        SessionColumn::Active => {
            if app.active_targets.contains(&session_target(session)) {
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

fn selected_report(app: &TopApp) -> Option<&AgentReport> {
    app.table_state
        .selected()
        .and_then(|selected| app.reports.get(selected))
}

fn session_target(session: &AgentSession) -> String {
    format!("{}:{}", session.kind.slug(), session.id)
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

fn format_tokens_compact(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format_compact(tokens, 1_000_000, "M")
    } else if tokens >= 1_000 {
        format_compact(tokens, 1_000, "K")
    } else {
        tokens.to_string()
    }
}

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

fn status_style(status: &str) -> Style {
    match status {
        "running" => Style::default().fg(Color::Green),
        "stopped" | "exited" => Style::default().fg(Color::Red),
        "sleeping" | "idle" => Style::default().fg(Color::Gray),
        _ => Style::default(),
    }
}

fn detail_line(label: &'static str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("{label:<8}"), Style::default().fg(METADATA_KEY)),
        Span::raw(value),
    ])
}

fn key_hints(hints: &[(&str, &str)]) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, (key, action)) in hints.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled("  •  ", Style::default().fg(MUTED)));
        }
        spans.push(Span::styled(
            (*key).to_owned(),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(format!(" {action}")));
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use anyhow::Result;
    use crossterm::event::{
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
    };
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::{Color, Modifier};

    use super::{
        BrowserMode, BrowserPurpose, DetailAction, SessionsApp, TopApp, coalesce_detail_events,
        draw_sessions, draw_top, handle_detail_event, handle_detail_key, session_columns,
        session_target,
    };
    use crate::AgentKind;
    use crate::process::{LiveAgent, ProcessSnapshot};
    use crate::session::{
        AgentSession, DeletionSummary, SessionDetail, SessionMessage, SessionMessageKind,
    };
    use crate::view::AgentReport;

    #[test]
    fn top_layout_stays_aligned_at_eighty_columns_with_details_open() {
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).expect("test terminal");
        let mut app = TopApp::new(vec![report()]);
        app.show_details = true;
        terminal
            .draw(|frame| draw_top(frame, &mut app))
            .expect("draw top");

        let screen = buffer_text(terminal.backend().buffer(), 80, 20);
        assert!(screen.contains("mena top"));
        assert!(screen.contains("codex:42"));
        assert!(screen.contains("Details"));
        assert!(screen.contains("q quit"));
        assert!(screen.lines().all(|line| line.chars().count() == 80));
    }

    #[test]
    fn top_refresh_can_transition_to_no_running_agents() {
        let mut app = TopApp::new(vec![report()]);

        app.replace(Vec::new());

        assert!(app.reports.is_empty());
        assert_eq!(app.table_state.selected(), None);
    }

    #[test]
    fn session_layout_displays_titles_and_filters_by_them() {
        let session = report().session.expect("fixture session");
        let mut app = SessionsApp::new(vec![session], BTreeSet::default(), BrowserPurpose::Manage);
        app.query = "rendering".to_owned();
        app.recompute_filter();
        assert_eq!(app.filtered.len(), 1);

        let mut terminal = Terminal::new(TestBackend::new(80, 18)).expect("test terminal");
        terminal
            .draw(|frame| draw_sessions(frame, &mut app))
            .expect("draw sessions");
        let screen = buffer_text(terminal.backend().buffer(), 80, 18);
        assert!(screen.contains("Fix terminal rendering"));
        assert!(screen.contains("d delete"));
        assert!(screen.lines().all(|line| line.chars().count() == 80));
    }

    #[test]
    fn session_target_is_first_and_visible_at_eighty_columns() {
        let mut session = report().session.expect("fixture session");
        session.id = "019fbd66-e95f-7dd2-b9b4-37a27a61c272".to_owned();
        let target = session_target(&session);
        let mut app = SessionsApp::new(vec![session], BTreeSet::default(), BrowserPurpose::Manage);
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
        let first = report().session.expect("first session");
        let mut second = first.clone();
        second.id = "second-session".to_owned();
        let mut app = SessionsApp::new(
            vec![first.clone(), second],
            BTreeSet::default(),
            BrowserPurpose::Manage,
        );
        app.open_detail(SessionDetail {
            session: first,
            messages: vec![SessionMessage {
                kind: SessionMessageKind::User,
                timestamp: None,
                model: None,
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
        let first = report().session.expect("first session");
        let mut second = first.clone();
        second.id = "second-session".to_owned();
        let mut app = SessionsApp::new(
            vec![first.clone(), second],
            BTreeSet::default(),
            BrowserPurpose::Manage,
        );
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
        let mut session = report().session.expect("fixture session");
        session.started_at = Some("2026-08-01T01:02:03Z".to_owned());
        session.cost_usd = Some(1.25);
        let mut app = SessionsApp::new(
            vec![session.clone()],
            BTreeSet::default(),
            BrowserPurpose::Manage,
        );
        app.open_detail(SessionDetail {
            session,
            messages: vec![
                SessionMessage {
                    kind: SessionMessageKind::User,
                    timestamp: Some("2026-08-01T01:02:04Z".to_owned()),
                    model: None,
                    content: "complete first question".to_owned(),
                },
                SessionMessage {
                    kind: SessionMessageKind::Assistant,
                    timestamp: Some("2026-08-01T01:02:05Z".to_owned()),
                    model: Some("gpt-5.5".to_owned()),
                    content: "complete first answer".to_owned(),
                },
                SessionMessage {
                    kind: SessionMessageKind::Assistant,
                    timestamp: Some("2026-08-01T01:02:06Z".to_owned()),
                    model: Some("gpt-5.6".to_owned()),
                    content: "complete second answer".to_owned(),
                },
            ],
        });
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).expect("test terminal");

        terminal
            .draw(|frame| draw_sessions(frame, &mut app))
            .expect("draw details");

        let screen = buffer_text(terminal.backend().buffer(), 100, 30);
        for expected in [
            "Session details",
            "Started",
            "2026-08-01T01:02:03Z",
            "Tokens",
            "125500000",
            "Cost",
            "$1.2500",
            "Conversation (3 messages)",
            "ASSISTANT · gpt-5.5",
            "ASSISTANT · gpt-5.6",
            "complete first question",
            "complete first answer",
            "complete second answer",
            "Shift+↑/↓ message",
            "c copy",
            "r resume",
            "e export",
            "Enter/Esc close",
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
    fn detail_messages_color_headers_and_bodies_by_primary_or_supporting_kind() {
        let session = report().session.expect("fixture session");
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
                content: format!("plain-body-{index}"),
            })
            .collect();
        let mut app = SessionsApp::new(
            vec![session.clone()],
            BTreeSet::default(),
            BrowserPurpose::Manage,
        );
        app.open_detail(SessionDetail { session, messages });
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
    fn detail_metadata_keys_are_pink_purple() {
        let session = report().session.expect("fixture session");
        let mut app = SessionsApp::new(
            vec![session.clone()],
            BTreeSet::default(),
            BrowserPurpose::Manage,
        );
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
        let session = report().session.expect("fixture session");
        let messages = (0..40)
            .map(|index| SessionMessage {
                kind: if index % 2 == 0 {
                    SessionMessageKind::User
                } else {
                    SessionMessageKind::Assistant
                },
                timestamp: None,
                model: None,
                content: format!("complete message number {index}"),
            })
            .collect();
        let mut app = SessionsApp::new(
            vec![session.clone()],
            BTreeSet::default(),
            BrowserPurpose::Manage,
        );
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
        let session = report().session.expect("fixture session");
        let wrapped = "aaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbb cccccccccccccccc\n".repeat(20);
        let messages = vec![SessionMessage {
            kind: SessionMessageKind::Assistant,
            timestamp: None,
            model: Some("gpt-5.6".to_owned()),
            content: format!("{wrapped}FINAL DETAIL CONTENT"),
        }];
        let mut app = SessionsApp::new(
            vec![session.clone()],
            BTreeSet::default(),
            BrowserPurpose::Manage,
        );
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
        let session = report().session.expect("fixture session");
        let wrapped = "aaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbb cccccccccccccccc\n".repeat(20);
        let messages = vec![SessionMessage {
            kind: SessionMessageKind::Assistant,
            timestamp: None,
            model: Some("gpt-5.6".to_owned()),
            content: format!("{wrapped}FINAL CONTENT AFTER RESIZE"),
        }];
        let mut app = SessionsApp::new(
            vec![session.clone()],
            BTreeSet::default(),
            BrowserPurpose::Manage,
        );
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
        let session = report().session.expect("fixture session");
        let mut content = "complete detail line\n".repeat(66_000);
        content.push_str("ABSOLUTE FINAL DETAIL LINE");
        let messages = vec![SessionMessage {
            kind: SessionMessageKind::Assistant,
            timestamp: None,
            model: Some("gpt-5.6".to_owned()),
            content,
        }];
        let mut app = SessionsApp::new(
            vec![session.clone()],
            BTreeSet::default(),
            BrowserPurpose::Manage,
        );
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
        let session = report().session.expect("fixture session");
        let messages = vec![
            SessionMessage {
                kind: SessionMessageKind::User,
                timestamp: None,
                model: None,
                content: "first user message".to_owned(),
            },
            SessionMessage {
                kind: SessionMessageKind::ToolCall,
                timestamp: None,
                model: None,
                content: "tool detail\n".repeat(15),
            },
            SessionMessage {
                kind: SessionMessageKind::ToolResult,
                timestamp: None,
                model: None,
                content: "tool result\n".repeat(15),
            },
            SessionMessage {
                kind: SessionMessageKind::Assistant,
                timestamp: None,
                model: Some("gpt-5.5".to_owned()),
                content: "assistant answer".to_owned(),
            },
        ];
        let mut app = SessionsApp::new(
            vec![session.clone()],
            BTreeSet::default(),
            BrowserPurpose::Manage,
        );
        app.open_detail(SessionDetail { session, messages });
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
        let session = report().session.expect("fixture session");
        let mut app = SessionsApp::new(
            vec![session.clone()],
            BTreeSet::default(),
            BrowserPurpose::Manage,
        );
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
    fn exporting_from_detail_keeps_selection_scroll_and_popup_open() {
        let first = report().session.expect("first session");
        let mut second = first.clone();
        second.id = "second-session".to_owned();
        let mut app = SessionsApp::new(
            vec![first.clone(), second],
            BTreeSet::default(),
            BrowserPurpose::Manage,
        );
        app.open_detail(SessionDetail {
            session: first,
            messages: Vec::new(),
        });
        app.detail_max_scroll = 20;
        app.detail_scroll = 7;
        let selected = app.table_state.selected();
        let mut export = |_detail: &SessionDetail| Ok(PathBuf::from("/tmp/session-export.md"));

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
            status.text == "Exported: /tmp/session-export.md"
                && status.style.fg == Some(Color::Green)
        }));
    }

    #[test]
    fn copying_from_detail_copies_the_complete_session_and_keeps_context() {
        let session = report().session.expect("fixture session");
        let mut app = SessionsApp::new(
            vec![session.clone()],
            BTreeSet::default(),
            BrowserPurpose::Manage,
        );
        app.open_detail(SessionDetail {
            session,
            messages: vec![SessionMessage {
                kind: SessionMessageKind::Assistant,
                timestamp: None,
                model: Some("gpt-5.6".to_owned()),
                content: "complete clipboard tail".to_owned(),
            }],
        });
        app.detail_max_scroll = 20;
        app.detail_scroll = 7;
        let selected = app.table_state.selected();
        let mut copied_tail = None;
        let mut copy = |detail: &SessionDetail| {
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
            status.text == "Copied complete detail to clipboard"
                && status.style.fg == Some(Color::Green)
        }));
    }

    #[test]
    fn failed_detail_copy_keeps_context_and_reports_a_red_error() {
        let session = report().session.expect("fixture session");
        let mut app = SessionsApp::new(
            vec![session.clone()],
            BTreeSet::default(),
            BrowserPurpose::Manage,
        );
        app.open_detail(SessionDetail {
            session,
            messages: Vec::new(),
        });
        app.detail_max_scroll = 20;
        app.detail_scroll = 7;
        let selected = app.table_state.selected();
        let mut copy =
            |_detail: &SessionDetail| -> Result<()> { anyhow::bail!("clipboard unavailable") };

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
        let session = report().session.expect("fixture session");
        let mut app = SessionsApp::new(
            vec![session.clone()],
            BTreeSet::default(),
            BrowserPurpose::Manage,
        );
        app.open_detail(SessionDetail {
            session,
            messages: Vec::new(),
        });
        app.detail_max_scroll = 20;
        app.detail_scroll = 7;
        let selected = app.table_state.selected();
        let mut export =
            |_detail: &SessionDetail| -> Result<PathBuf> { anyhow::bail!("permission denied") };

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
        let session = report().session.expect("fixture session");
        let target = session_target(&session);
        let mut app = SessionsApp::new(
            vec![session],
            BTreeSet::from([target]),
            BrowserPurpose::Manage,
        );

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
        let session = report().session.expect("fixture session");
        let mut duplicate = session.clone();
        duplicate.path = PathBuf::from("/tmp/duplicate-session.jsonl");
        let mut app = SessionsApp::new(
            vec![session.clone(), duplicate],
            BTreeSet::new(),
            BrowserPurpose::Manage,
        );

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

    fn report() -> AgentReport {
        AgentReport {
            agent: LiveAgent {
                kind: AgentKind::Codex,
                process: ProcessSnapshot {
                    pid: 42,
                    parent_pid: Some(1),
                    executable: PathBuf::from("/opt/bin/codex"),
                    command: vec!["codex".to_owned()],
                    cwd: Some(PathBuf::from("/work/project")),
                    started_at: 1,
                    run_time: 62,
                    cpu_percent: 1.0,
                    memory_bytes: 2_000_000,
                    status: "running".to_owned(),
                },
            },
            session: Some(AgentSession {
                kind: AgentKind::Codex,
                id: "session-id".to_owned(),
                title: Some("Fix terminal rendering".to_owned()),
                project: Some(PathBuf::from("/work/project")),
                path: PathBuf::from("/tmp/session.jsonl"),
                started_at: None,
                updated_at: 1,
                tokens: Some(125_500_000),
                cost_usd: None,
            }),
        }
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
}
