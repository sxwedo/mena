use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Cell, Clear, Paragraph, Row, Table, TableState, Wrap};
use unicode_width::UnicodeWidthChar;

use super::app::{McpApp, McpDetailLayout, McpFocus};
use crate::mcp::McpRegistration;
use crate::tui::common::{
    UI, app_header, badge, centered_rect, header_inner, key_hints, panel_block, panel_title,
    render_canvas, render_header_frame, responsive_key_hints, scroll_meter, selection_style,
    table_header_style,
};

pub(crate) fn draw_mcp(frame: &mut Frame<'_>, app: &mut McpApp) {
    render_canvas(frame);
    let areas = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(8),
        Constraint::Length(1),
    ])
    .split(frame.area());

    render_header(frame, areas[0], app);
    if app.full_screen_detail {
        render_detail(frame, areas[1], app);
    } else if areas[1].width < 100 {
        let rows = Layout::vertical([Constraint::Percentage(48), Constraint::Percentage(52)])
            .split(areas[1]);
        render_list(frame, rows[0], app);
        render_detail(frame, rows[1], app);
    } else {
        let list_percent = if areas[1].width >= 130 { 45 } else { 42 };
        let columns = Layout::horizontal([
            Constraint::Percentage(list_percent),
            Constraint::Percentage(100 - list_percent),
        ])
        .split(areas[1]);
        render_list(frame, columns[0], app);
        render_detail(frame, columns[1], app);
    }
    render_footer(frame, areas[2]);
    if app.pending_delete.is_some() {
        render_delete_confirmation(frame, app);
    }
}

fn render_header(frame: &mut Frame<'_>, area: Rect, app: &McpApp) {
    render_header_frame(frame, area, " MCP registrations ");
    let context = if app.is_searching {
        vec![
            badge("Filter", UI.amber),
            Span::styled("  Query: ", Style::default().fg(UI.muted)),
            Span::styled(
                format!("{}▌", app.query),
                Style::default().fg(UI.amber).bold(),
            ),
            Span::styled(
                format!(
                    "  {}/{} visible",
                    app.visible.len(),
                    app.registrations.len()
                ),
                Style::default().fg(UI.muted),
            ),
        ]
    } else if let Some(notice) = &app.notice {
        vec![
            badge(
                if notice.error { "Error" } else { "Done" },
                if notice.error { UI.danger } else { UI.success },
            ),
            Span::raw("  "),
            Span::styled(
                notice.message.clone(),
                Style::default().fg(if notice.error {
                    UI.danger
                } else {
                    UI.text_soft
                }),
            ),
        ]
    } else {
        let probe = app.probe_in_progress.map_or_else(
            || format!("{} registrations", app.registrations.len()),
            |index| {
                format!(
                    "{} {}",
                    if app.exit_after_probe {
                        "finishing probe before exit:"
                    } else {
                        "probing"
                    },
                    app.registrations
                        .get(index)
                        .map_or("registration", |registration| registration
                            .selector
                            .as_str())
                )
            },
        );
        vec![
            badge("Static", UI.cyan),
            Span::styled(format!("  {probe}"), Style::default().fg(UI.text_soft)),
        ]
    };
    frame.render_widget(
        Paragraph::new(app_header("MCP", context)),
        header_inner(area),
    );
}

fn render_list(frame: &mut Frame<'_>, area: Rect, app: &McpApp) {
    let compact = area.width < 54;
    let mut rows = Vec::with_capacity(app.visible.len().saturating_mul(2));
    let mut selected_row = None;
    let mut current_provider = None::<&str>;
    for (visible_index, catalog_index) in app.visible.iter().enumerate() {
        let registration = &app.registrations[*catalog_index];
        if current_provider != Some(registration.provider.as_str()) {
            current_provider = Some(&registration.provider);
            let count = app
                .visible
                .iter()
                .filter(|index| app.registrations[**index].provider == registration.provider)
                .count();
            let mut cells = vec![Cell::from(format!("{} · {count}", registration.provider))];
            cells.resize_with(if compact { 2 } else { 4 }, || Cell::from(""));
            rows.push(
                Row::new(cells).style(
                    Style::default()
                        .fg(UI.text_soft)
                        .bg(UI.panel_alt)
                        .add_modifier(Modifier::BOLD),
                ),
            );
        }
        if visible_index == app.selected_index {
            selected_row = Some(rows.len());
        }
        let state = registration_state(registration);
        let state_style = match state {
            "Enabled" => Style::default().fg(UI.success),
            "Disabled" => Style::default().fg(UI.muted),
            _ => Style::default().fg(UI.danger),
        };
        if compact {
            rows.push(Row::new(vec![
                Cell::from(format!("{}/{}", registration.scope, registration.name)),
                Cell::from(Span::styled(state, state_style)),
            ]));
        } else {
            rows.push(Row::new(vec![
                Cell::from(registration.name.clone()),
                Cell::from(registration.scope.clone()),
                Cell::from(registration.transport.as_str()),
                Cell::from(Span::styled(state, state_style)),
            ]));
        }
    }
    let widths: Vec<Constraint> = if compact {
        vec![Constraint::Min(12), Constraint::Length(9)]
    } else {
        vec![
            Constraint::Min(16),
            Constraint::Length(9),
            Constraint::Length(16),
            Constraint::Length(9),
        ]
    };
    let headers = if compact {
        vec!["Scope / name", "Status"]
    } else {
        vec!["Name", "Scope", "Transport", "Status"]
    };
    let active = app.focus == McpFocus::List && !app.full_screen_detail;
    let title = panel_title(
        "Registrations",
        Some(format!(
            "{} visible · {} total",
            app.visible.len(),
            app.registrations.len()
        )),
        active,
    );
    let table = Table::new(rows, widths)
        .header(Row::new(headers).style(table_header_style()))
        .row_highlight_style(selection_style())
        .highlight_symbol("> ")
        .block(panel_block(title, active));
    let mut state = TableState::default();
    state.select(selected_row);
    frame.render_stateful_widget(table, area, &mut state);
}

fn render_detail(frame: &mut Frame<'_>, area: Rect, app: &mut McpApp) {
    let selected = app.selected_registration().map(|registration| {
        (
            app.selected_catalog_index()
                .expect("selected registration index"),
            registration.selector.clone(),
            app.probe_in_progress == app.selected_catalog_index(),
        )
    });
    let active = app.focus == McpFocus::Detail || app.full_screen_detail;
    let title = selected.as_ref().map_or_else(
        || panel_title("Details", None, active),
        |(_, selector, probing)| {
            panel_title(
                "Details",
                Some(format!(
                    "{selector}{} · {}",
                    if *probing { " · probing" } else { "" },
                    scroll_meter(app.detail_scroll, app.detail_max_scroll, 5)
                )),
                active,
            )
        },
    );
    let mut block = panel_block(title, active);
    if app.full_screen_detail {
        block = block.border_style(Style::default().fg(UI.signal));
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some((registration_index, _, probing)) = selected else {
        frame.render_widget(
            Paragraph::new("No MCP registration selected")
                .style(Style::default().fg(UI.muted).bg(UI.panel)),
            inner,
        );
        app.detail_scroll = 0;
        app.detail_max_scroll = 0;
        return;
    };

    let layout_is_stale = app.detail_layout.as_ref().is_none_or(|layout| {
        layout.registration_index != registration_index || layout.width != inner.width
    });
    if layout_is_stale {
        let probe_error = app.selected_probe_error().map(str::to_owned);
        let mut content = app.selected_detail_text().unwrap_or_default().to_owned();
        if probing {
            content.push_str("\nLive metadata probe in progress (tools are never called).\n");
        }
        if let Some(error) = probe_error {
            content.push_str("\nProbe request failed: ");
            content.push_str(&error);
            content.push('\n');
        }
        app.detail_layout = Some(McpDetailLayout {
            registration_index,
            width: inner.width,
            lines: style_and_wrap_detail(&content, inner.width),
        });
    }

    let layout = app.detail_layout.as_ref().expect("detail layout");
    app.detail_max_scroll = layout.lines.len().saturating_sub(usize::from(inner.height));
    app.detail_scroll = app.detail_scroll.min(app.detail_max_scroll);
    let visible = layout
        .lines
        .iter()
        .skip(app.detail_scroll)
        .take(usize::from(inner.height))
        .cloned()
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(Text::from(visible)), inner);
}

fn render_footer(frame: &mut Frame<'_>, area: Rect) {
    frame.render_widget(
        Paragraph::new(responsive_key_hints(
            area.width,
            &[
                ("o", "open"),
                ("e", "edit"),
                ("d", "delete"),
                ("p", "probe"),
                ("/", "filter"),
                ("Enter", "focus"),
                ("q", "back"),
            ],
            &[
                ("p", "probe"),
                ("/", "filter"),
                ("Enter", "focus"),
                ("q", "back"),
            ],
        ))
        .alignment(Alignment::Center),
        area,
    );
}

fn render_delete_confirmation(frame: &mut Frame<'_>, app: &McpApp) {
    let Some(registration) = app
        .pending_delete
        .and_then(|index| app.registrations.get(index))
    else {
        return;
    };
    let area = centered_rect(frame.area(), 88, 10);
    frame.render_widget(Clear, area);
    let block = panel_block(
        panel_title(
            "Delete registration",
            Some("This cannot be undone".to_owned()),
            true,
        ),
        true,
    )
    .border_style(Style::default().fg(UI.danger));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let lines = vec![
        Line::from(Span::styled(
            "Permanently remove this registration from its native config?",
            Style::default().fg(UI.danger).bold(),
        )),
        Line::default(),
        Line::from(vec![
            Span::styled("Target: ", Style::default().fg(UI.muted)),
            Span::styled(
                registration.selector.clone(),
                Style::default().fg(UI.text).bold(),
            ),
        ]),
        Line::from(vec![
            Span::styled("Source: ", Style::default().fg(UI.muted)),
            Span::styled(
                registration.source.display().to_string(),
                Style::default().fg(UI.text_soft),
            ),
        ]),
        Line::default(),
        key_hints(&[("y", "delete permanently"), ("n/Esc", "cancel")]),
    ];
    frame.render_widget(
        Paragraph::new(Text::from(lines))
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true }),
        inner,
    );
}

const fn registration_state(registration: &McpRegistration) -> &'static str {
    if !registration.valid {
        "Invalid"
    } else if registration.enabled {
        "Enabled"
    } else {
        "Disabled"
    }
}

fn style_and_wrap_detail(content: &str, width: u16) -> Vec<Line<'static>> {
    let width = usize::from(width.max(1));
    let mut lines = Vec::new();
    for logical_line in content.lines() {
        let style = detail_line_style(logical_line);
        let mut current = String::new();
        let mut current_width = 0;
        for character in logical_line.chars() {
            if character == '\t' {
                for _ in 0..4 {
                    push_wrapped_character(
                        &mut lines,
                        &mut current,
                        &mut current_width,
                        ' ',
                        width,
                        style,
                    );
                }
            } else {
                push_wrapped_character(
                    &mut lines,
                    &mut current,
                    &mut current_width,
                    character,
                    width,
                    style,
                );
            }
        }
        if current.is_empty() {
            lines.push(Line::default());
        } else {
            lines.push(Line::from(Span::styled(current, style)));
        }
    }
    if lines.is_empty() {
        lines.push(Line::default());
    }
    lines
}

fn push_wrapped_character(
    lines: &mut Vec<Line<'static>>,
    current: &mut String,
    current_width: &mut usize,
    character: char,
    max_width: usize,
    style: Style,
) {
    let width = UnicodeWidthChar::width(character).unwrap_or_default();
    if !current.is_empty() && current_width.saturating_add(width) > max_width {
        lines.push(Line::from(Span::styled(std::mem::take(current), style)));
        *current_width = 0;
    }
    current.push(character);
    *current_width = current_width.saturating_add(width);
}

fn detail_line_style(line: &str) -> Style {
    if line.starts_with("MCP ") {
        Style::default().fg(UI.text).add_modifier(Modifier::BOLD)
    } else if line == "Static registration metadata"
        || line.starts_with("Runtime metadata:")
        || line.starts_with("Runtime tools:")
        || line.starts_with("Runtime prompts:")
        || line.starts_with("Runtime resources:")
    {
        Style::default().fg(UI.signal).add_modifier(Modifier::BOLD)
    } else if line.contains("Error:") || line.starts_with("Probe request failed:") {
        Style::default().fg(UI.danger)
    } else if line.contains("Warning:") {
        Style::default().fg(UI.amber)
    } else {
        Style::default().fg(UI.text_soft)
    }
}
