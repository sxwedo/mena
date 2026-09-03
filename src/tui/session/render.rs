use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table, Wrap};

use super::app::*;
use crate::session::{AgentSession, DetailScope, SessionDetail, SessionMessageKind};
use crate::tui::common::{
    SessionDetailTheme, UI, app_header, badge, centered_rect, header_inner, panel_block,
    panel_title, render_canvas, render_header_frame, responsive_key_hints, scroll_meter,
    selection_style, table_header_style, themed_key_hints, thinking_orb_spans,
};
use crate::view::{
    TOOL_TOKEN_ACCOUNTING_NOTE, format_duration, format_metric_error, format_model_usage_summary,
    format_response_header_metrics, format_response_summary, format_token_breakdown,
    format_tool_summary,
};

pub(crate) fn draw_sessions(frame: &mut Frame<'_>, app: &mut SessionsApp, _tick: usize) {
    render_canvas(frame);
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
    if app.mode == BrowserMode::Rename {
        render_rename_header(frame, area, app);
        return;
    }
    let (state_color, state) = if app.mode == BrowserMode::Search {
        (UI.amber, "Filter")
    } else {
        (UI.cyan, "All")
    };
    let query = if app.query.is_empty() {
        "All sessions"
    } else {
        &app.query
    };

    render_header_frame(frame, area, " Sessions ");

    let query_style = if app.mode == BrowserMode::Search {
        Style::default().fg(UI.amber).add_modifier(Modifier::BOLD)
    } else if app.query.is_empty() {
        Style::default().fg(UI.muted)
    } else {
        Style::default().fg(UI.text)
    };

    frame.render_widget(
        Paragraph::new(app_header(
            "Sessions",
            vec![
                badge(state, state_color),
                Span::styled("  Search: ", Style::default().fg(UI.muted)),
                Span::styled(
                    format!(
                        "{query}{}",
                        if app.mode == BrowserMode::Search {
                            "▌"
                        } else {
                            ""
                        }
                    ),
                    query_style,
                ),
                Span::styled(
                    format!("  {}/{} visible", app.filtered.len(), app.sessions.len()),
                    Style::default().fg(UI.muted),
                ),
                // In batch mode the amber mark count is the headline state:
                // every other action is locked until marks are cleared.
                Span::styled(
                    format!("  ◆ {} marked for deletion", app.marked_count()),
                    Style::default()
                        .fg(if app.marked_count() > 0 {
                            UI.amber
                        } else {
                            UI.grid
                        })
                        .add_modifier(if app.marked_count() > 0 {
                            Modifier::BOLD
                        } else {
                            Modifier::empty()
                        }),
                ),
            ],
        )),
        header_inner(area),
    );
}

fn render_rename_header(frame: &mut Frame<'_>, area: Rect, app: &SessionsApp) {
    render_header_frame(frame, area, " Sessions ");
    frame.render_widget(
        Paragraph::new(app_header(
            "Sessions",
            vec![
                badge("Rename", UI.amber),
                Span::styled("  Title: ", Style::default().fg(UI.muted)),
                Span::styled(
                    format!("{}▌", app.rename_draft),
                    Style::default().fg(UI.amber).add_modifier(Modifier::BOLD),
                ),
            ],
        )),
        header_inner(area),
    );
}

fn render_session_table_widget(frame: &mut Frame<'_>, area: Rect, app: &mut SessionsApp) {
    let columns = session_columns(area.width);
    let header = Row::new(columns.iter().map(|column| Cell::from(column.label)))
        .style(table_header_style())
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
                // Column 0 is the narrow mark column; the project line starts
                // at the TARGET column and spans every remaining column, so
                // the full project path renders instead of being clipped to
                // the narrow TARGET width.
                let icon = if *collapsed { "▸" } else { "▾" };
                let count_label = if *count == 1 {
                    "(1 session)".to_owned()
                } else {
                    format!("({count} sessions)")
                };
                let line = Line::from(vec![
                    Span::styled(
                        icon,
                        Style::default().fg(UI.signal).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" "),
                    Span::styled(
                        project.as_str(),
                        Style::default().fg(UI.text).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::styled(count_label, Style::default().fg(UI.muted)),
                ]);
                let remaining_columns =
                    u16::try_from(columns.len().saturating_sub(1)).unwrap_or(u16::MAX);
                rows.push(Row::new(vec![
                    Cell::from(""),
                    Cell::from(line).column_span(remaining_columns),
                ]));
            }
            DisplayRow::Session { session_index } => {
                rows.push(session_row(*session_index));
            }
        }
    }

    let grouping_label = app.grouping.label();
    let table_title = panel_title(
        "Sessions",
        Some(format!(
            "{} shown · {} saved · Group: {}",
            app.filtered.len(),
            app.sessions.len(),
            grouping_label
        )),
        true,
    );

    let table = Table::new(rows, columns.iter().map(|column| column.constraint))
        .header(header)
        .block(panel_block(table_title, true))
        .column_spacing(1)
        .row_highlight_style(selection_style())
        .highlight_symbol("▎ ");
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

/// Group rows span the columns after the mark column and show the full
/// project path (the grouping key); the sessions below them keep compact
/// short targets.
fn render_session_detail_popup(frame: &mut Frame<'_>, app: &mut SessionsApp) {
    if !matches!(app.mode, BrowserMode::Detail | BrowserMode::DetailSearch) {
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
    let total_h = app
        .detail_layout
        .as_ref()
        .map_or(0, DetailLayoutCache::total_height);
    let title_text = Line::styled(
        format!(
            " Session details  {} · line {}/{total_h} ",
            scroll_meter(app.detail_scroll, max_scroll, 6),
            app.detail_scroll + 1,
        ),
        Style::default()
            .fg(theme.popup_title)
            .add_modifier(Modifier::BOLD),
    );
    let block = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
        .style(Style::default().bg(UI.panel).fg(UI.text))
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
    let (text, local_scroll) = {
        let layout = app
            .detail_layout
            .as_ref()
            .expect("detail layout was initialized");
        let (start, end, local_scroll) =
            layout.visible_span(app.detail_scroll, usize::from(areas[0].height));
        if end <= start {
            (Text::default(), local_scroll)
        } else {
            // Search matches carry a quiet band; the focused match glows.
            let (match_lines, focus_line) = app.detail_search.as_ref().map_or_else(
                || (&[][..], None),
                |search| {
                    (
                        search.match_lines.as_slice(),
                        search.match_lines.get(search.cursor).copied(),
                    )
                },
            );
            let lines = layout.lines[start..end]
                .iter()
                .enumerate()
                .map(|(offset, line)| {
                    let global = start + offset;
                    let style = if focus_line == Some(global) {
                        Style::default().bg(UI.amber)
                    } else if match_lines.contains(&global) {
                        Style::default().bg(UI.grid)
                    } else {
                        return line.clone();
                    };
                    line.clone().patch_style(style)
                })
                .collect::<Vec<Line<'static>>>();
            (Text::from(lines), local_scroll)
        }
    };
    let paragraph = Paragraph::new(text).wrap(Wrap { trim: false });

    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    frame.render_widget(paragraph.scroll((local_scroll, 0)), areas[0]);
    render_detail_status_bar(frame, areas[1], app.detail_status.as_ref(), theme);
    render_detail_footer(frame, areas[2], app);
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

fn render_detail_footer(frame: &mut Frame<'_>, area: Rect, app: &SessionsApp) {
    let theme = app.detail_theme;
    if app.mode == BrowserMode::DetailSearch
        && let Some(search) = app.detail_search.as_ref()
    {
        let query_line = Line::from(vec![
            Span::styled(
                "/",
                Style::default()
                    .fg(theme.footer_key)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(
                    "{}{}",
                    search.query,
                    if app.mode == BrowserMode::DetailSearch {
                        "▌"
                    } else {
                        ""
                    }
                ),
                Style::default().fg(theme.footer_text),
            ),
            Span::styled(
                format!(
                    "  ·  {} match{}  ·  Enter keep · Esc cancel",
                    search.match_lines.len(),
                    if search.match_lines.len() == 1 {
                        ""
                    } else {
                        "es"
                    }
                ),
                Style::default().fg(UI.muted),
            ),
        ]);
        frame.render_widget(
            Paragraph::new(query_line).alignment(Alignment::Center),
            area,
        );
        return;
    }
    // A committed search advertises its position and the n/N jump keys.
    let search_suffix = app.detail_search.as_ref().map(|search| {
        let position = if search.match_lines.is_empty() {
            0
        } else {
            search.cursor + 1
        };
        format!(
            "  ·  /{} {}/{} · n/N jump · Esc clear",
            search.query,
            position,
            search.match_lines.len()
        )
    });
    if let Some(suffix) = search_suffix {
        let hints = if area.width >= 108 {
            themed_key_hints(
                &[
                    ("n/N", "jump"),
                    ("p", "chat"),
                    ("c", "copy"),
                    ("Esc", "clear"),
                ],
                theme.footer_key,
                theme.footer_text,
                theme.footer_separator,
            )
        } else {
            themed_key_hints(
                &[("n/N", "jump"), ("Esc", "clear")],
                theme.footer_key,
                theme.footer_text,
                theme.footer_separator,
            )
        };
        let line = Line::from(
            hints
                .spans
                .into_iter()
                .chain([Span::styled(suffix, Style::default().fg(UI.muted))])
                .collect::<Vec<Span<'static>>>(),
        );
        frame.render_widget(Paragraph::new(line).alignment(Alignment::Center), area);
        return;
    }
    frame.render_widget(
        Paragraph::new(if area.width >= 108 {
            themed_key_hints(
                &[
                    ("Shift+↑/↓", "msg"),
                    ("p", "chat"),
                    ("Shift+P", "all"),
                    ("c", "copy"),
                    ("r", "resume"),
                    ("R", "handoff"),
                    ("e", "export"),
                    ("/", "find"),
                    ("Esc", "close"),
                ],
                theme.footer_key,
                theme.footer_text,
                theme.footer_separator,
            )
        } else {
            themed_key_hints(
                &[
                    ("p/P", "scope"),
                    ("c", "copy"),
                    ("r", "resume"),
                    ("R", "handoff"),
                    ("/", "find"),
                    ("Esc", "close"),
                ],
                theme.footer_key,
                theme.footer_text,
                theme.footer_separator,
            )
        })
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
        native_resume_line(session, theme),
        Line::from(""),
    ]
}

/// The provider-native resume argv rendered as one copyable shell line, so a
/// user can resume outside mena without reconstructing the flags.
fn native_resume_line(session: &AgentSession, theme: SessionDetailTheme) -> Line<'static> {
    let command = crate::session::native_resume_command(&session.kind, &session.id)
        .ok()
        .map(|spec| {
            if spec.args.is_empty() {
                spec.program
            } else {
                format!("{} {}", spec.program, spec.args.join(" "))
            }
        });
    session_detail_line("Resume", command.unwrap_or_else(|| "-".to_owned()), theme)
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
        || footer_for_status(app, area.width),
        |progress| searching_footer_line(progress, app),
    );
    frame.render_widget(Paragraph::new(footer).alignment(Alignment::Center), area);
}

fn footer_for_status(app: &SessionsApp, width: u16) -> Line<'static> {
    app.status.as_ref().map_or_else(
        || session_key_hints(app, width),
        |status| Line::from(Span::styled(status.text.clone(), status.style)),
    )
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

/// Footer hints for the session browser. `Space` toggles multi-select marks;
/// with marks present the browser is in batch mode: only marking and deleting
/// are offered, and every other action is locked until the marks are cleared
/// with Esc.
fn session_key_hints(app: &SessionsApp, width: u16) -> Line<'static> {
    if app.mode == BrowserMode::Rename {
        return responsive_key_hints(
            width,
            &[("Enter", "save"), ("Esc", "cancel")],
            &[("Enter", "save"), ("Esc", "cancel")],
        );
    }
    if app.marked_count() > 0 {
        return responsive_key_hints(
            width,
            &[
                ("d", "delete marked"),
                ("Space", "mark"),
                ("Esc", "clear"),
                ("q", "quit"),
            ],
            &[("d", "delete"), ("Space", "mark"), ("Esc", "clear")],
        );
    }
    responsive_key_hints(
        width,
        &[
            ("Space", "mark"),
            ("/", "filter"),
            ("Enter", "details"),
            ("t", "rename"),
            ("r", "resume"),
            ("R", "handoff"),
            ("d", "delete"),
            ("q", "quit"),
        ],
        &[
            ("Space", "mark"),
            ("/", "filter"),
            ("Enter", "details"),
            ("r", "resume"),
            ("R", "handoff"),
            ("q", "quit"),
        ],
    )
}

fn render_delete_confirmation(frame: &mut Frame<'_>, app: &SessionsApp) {
    if app.mode != BrowserMode::ConfirmDelete {
        return;
    }
    let targets = &app.confirm_delete_targets;
    if targets.is_empty() {
        return;
    }
    let count = targets.len();
    let batch = count > 1;

    // Spell out exactly what will be removed: up to five targets plus a
    // remainder count, so the user confirms a list — not just a number.
    let shown: Vec<&AgentSession> = targets.iter().take(5).collect();
    let remaining = count - shown.len();

    let mut lines: Vec<Line<'static>> = vec![
        Line::from(Span::styled(
            if batch {
                format!("DELETE {count} SESSIONS")
            } else {
                "DELETE THIS SESSION".to_owned()
            },
            Style::default().fg(UI.danger).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];
    for target in &shown {
        lines.push(Line::from(Span::styled(
            target.target(),
            Style::default().fg(UI.text),
        )));
    }
    if remaining > 0 {
        lines.push(Line::from(Span::styled(
            format!("… and {remaining} more"),
            Style::default().fg(UI.muted),
        )));
    }
    if !batch && let Some(title) = targets[0].title.clone() {
        lines.push(Line::from(Span::styled(
            title,
            Style::default().fg(UI.muted),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Native session files and provider indexes are removed. This cannot be undone.",
        Style::default().fg(UI.amber),
    )));
    lines.push(Line::from(""));
    // Imperative key guidance: name the key, the action, and the outcome.
    lines.push(Line::from(vec![
        Span::styled("Press ", Style::default().fg(UI.muted)),
        Span::styled(
            "y",
            Style::default().fg(UI.danger).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "  delete {} permanently",
                if batch {
                    format!("all {count} sessions")
                } else {
                    "this session".to_owned()
                }
            ),
            Style::default().fg(UI.text),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Press ", Style::default().fg(UI.muted)),
        Span::styled(
            "n",
            Style::default().fg(UI.success).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" or ", Style::default().fg(UI.muted)),
        Span::styled(
            "Esc",
            Style::default().fg(UI.success).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "  keep everything, nothing is deleted",
            Style::default().fg(UI.text),
        ),
    ]));

    // +4 covers the two border rows plus wrap headroom for the warning line
    // on terminals narrower than the popup's preferred width.
    let height = u16::try_from(lines.len() + 4).unwrap_or(u16::MAX);
    let popup = centered_rect(frame.area(), 72, height);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true })
            .block(
                panel_block(
                    panel_title(
                        "Confirm deletion",
                        Some("cannot be undone".to_owned()),
                        true,
                    ),
                    true,
                )
                .border_style(Style::default().fg(UI.danger)),
            ),
        popup,
    );
}

pub(crate) fn session_columns(width: u16) -> Vec<Column<SessionColumn>> {
    if width >= 120 {
        vec![
            column(SessionColumn::Marked, "", Constraint::Length(1)),
            column(SessionColumn::Target, "Target", Constraint::Length(20)),
            column(SessionColumn::Active, "", Constraint::Length(1)),
            column(SessionColumn::Agent, "Agent", Constraint::Length(12)),
            column(SessionColumn::Project, "Project", Constraint::Length(14)),
            column(SessionColumn::Title, "Title / summary", Constraint::Min(18)),
            column(SessionColumn::Updated, "Updated", Constraint::Length(11)),
        ]
    } else if width >= 80 {
        vec![
            column(SessionColumn::Marked, "", Constraint::Length(1)),
            column(SessionColumn::Target, "Target", Constraint::Length(20)),
            column(SessionColumn::Active, "", Constraint::Length(1)),
            column(SessionColumn::Title, "Title / summary", Constraint::Min(18)),
        ]
    } else {
        vec![
            column(SessionColumn::Marked, "", Constraint::Length(1)),
            column(SessionColumn::Target, "Target", Constraint::Length(20)),
            column(SessionColumn::Active, "", Constraint::Length(1)),
            column(SessionColumn::Title, "Title / summary", Constraint::Min(12)),
        ]
    }
}

pub(crate) fn session_cell(
    session: &AgentSession,
    column: SessionColumn,
    app: &SessionsApp,
) -> Cell<'static> {
    match column {
        SessionColumn::Marked => {
            if app.marked_targets.contains(&session.target()) {
                Cell::from(Span::styled(
                    "◆",
                    Style::default().fg(UI.amber).add_modifier(Modifier::BOLD),
                ))
            } else {
                Cell::from(Span::styled("·", Style::default().fg(UI.grid)))
            }
        }
        SessionColumn::Active => {
            if app.active_targets.contains(&session.target()) {
                Cell::from(Span::styled("●", Style::default().fg(UI.success)))
            } else {
                Cell::from("")
            }
        }
        SessionColumn::Agent => {
            let color = match session.kind {
                crate::process::AgentKind::ClaudeCode => UI.amber,
                crate::process::AgentKind::OhMyPi
                | crate::process::AgentKind::GeminiCli
                | crate::process::AgentKind::OpenCode => UI.cyan,
                crate::process::AgentKind::Pi => UI.violet,
                crate::process::AgentKind::Codex => UI.success,
                crate::process::AgentKind::Grok => UI.text_soft,
                _ => UI.text,
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
                Style::default().fg(UI.success).add_modifier(Modifier::BOLD)
            } else if age_secs < 86400 {
                Style::default().fg(UI.amber)
            } else if age_secs < 7 * 86400 {
                Style::default().fg(UI.text)
            } else {
                Style::default().fg(UI.muted)
            };
            Cell::from(Span::styled(age_str, style))
        }
        SessionColumn::Target => {
            let target = session.short_target();
            Cell::from(Span::styled(target, Style::default().fg(UI.signal)))
        }
        SessionColumn::Project => {
            let label = project_label(session.project.as_deref());
            let style = if label == "-" {
                Style::default().fg(UI.muted)
            } else {
                Style::default().fg(UI.text_soft)
            };
            Cell::from(Span::styled(label, style))
        }
        SessionColumn::Title => {
            let (title, is_placeholder) = session
                .title
                .as_ref()
                .map_or_else(|| ("(untitled)".to_owned(), true), |t| (t.clone(), false));
            let style = if is_placeholder {
                Style::default().fg(UI.muted)
            } else {
                Style::default().fg(UI.text)
            };
            Cell::from(Span::styled(title, style))
        }
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
            format!("{label:<10}: "),
            Style::default().fg(theme.metadata_key),
        ),
        Span::styled(value, Style::default().fg(theme.metadata_value)),
    ])
}
