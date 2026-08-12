use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table, Wrap};

use super::app::*;
use crate::session::{AgentSession, DetailScope, SessionDetail, SessionMessageKind};
use crate::tui::common::{
    ACCENT, MUTED, SessionDetailTheme, centered_rect, key_hints, render_border_beam,
    themed_key_hints, thinking_orb_spans,
};
use crate::view::{
    TOOL_TOKEN_ACCOUNTING_NOTE, format_duration, format_metric_error, format_model_usage_summary,
    format_response_header_metrics, format_response_summary, format_token_breakdown,
    format_tool_summary,
};

pub(crate) fn draw_sessions(frame: &mut Frame<'_>, app: &mut SessionsApp, tick: usize) {
    let areas = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(5),
        Constraint::Length(1),
    ])
    .split(frame.area());

    render_session_search(frame, areas[0], app, tick);
    render_session_table_widget(frame, areas[1], app);
    render_session_footer(frame, areas[2], app);
    render_session_detail_popup(frame, app);
    render_delete_confirmation(frame, app);
}

fn render_session_search(frame: &mut Frame<'_>, area: Rect, app: &SessionsApp, tick: usize) {
    let (beam_color, title) = if app.mode == BrowserMode::Search {
        (
            Color::Yellow,
            " Search — type to filter, Enter apply, Esc clear ",
        )
    } else {
        (Color::Rgb(56, 189, 248), " Search — press / to filter ")
    };
    let query = if app.query.is_empty() {
        "All sessions"
    } else {
        &app.query
    };

    render_border_beam(frame, area, tick, title, MUTED, beam_color);

    let query_style = if app.mode == BrowserMode::Search {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else if app.query.is_empty() {
        Style::default().fg(MUTED)
    } else {
        Style::default().fg(Color::White)
    };

    frame.render_widget(
        Paragraph::new(query).style(query_style),
        Block::new().inner(area),
    );
}

pub(crate) fn format_project_display_path(path_str: &str, max_len: usize) -> String {
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
        .block(
            Block::new()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(table_title),
        )
        .column_spacing(1)
        .row_highlight_style(
            Style::default()
                .bg(Color::Rgb(40, 44, 52))
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");
    frame.render_stateful_widget(table, area, &mut app.table_state);
}

/// Stable label used as the grouping key for project grouping. Sessions without
/// a project land in a dedicated bucket so they still group together.
pub(crate) fn session_project_label(session: &AgentSession) -> String {
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
    let max_scroll = app.detail_max_scroll;
    let scroll_percent = (app.detail_scroll * 100)
        .checked_div(max_scroll)
        .unwrap_or(100);
    let total_h = app
        .detail_layout
        .as_ref()
        .map_or(0, DetailLayoutCache::total_height);
    let title_text = format!(
        " Session details [Scroll: {scroll_percent}% | Line {}/{total_h}] ",
        app.detail_scroll + 1,
    );
    let block = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
        .title_style(Style::default().fg(theme.popup_title))
        .title(title_text);
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

pub(crate) fn fragment_detail_line(line: Line<'static>) -> Vec<Line<'static>> {
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

pub(crate) fn session_detail_content(
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
    let tick = (progress.started.elapsed().as_millis() / 80) as usize;
    let text = search_progress_text(progress, app.sessions.len());
    Line::from(thinking_orb_spans(tick, &text))
}

pub(crate) fn search_progress_text(progress: &InProgressSearch, total: usize) -> String {
    format!(
        "Scanning transcripts — {scanned}/{total} scanned, {hits} match{plural} (Esc cancel)",
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
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::Red))
                    .title(" Confirm Deletion "),
            ),
        popup,
    );
}

pub(crate) fn session_columns(width: u16) -> Vec<Column<SessionColumn>> {
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

pub(crate) fn session_cell(
    session: &AgentSession,
    column: SessionColumn,
    app: &SessionsApp,
) -> Cell<'static> {
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
