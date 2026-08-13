use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table, TableState,
};
use unicode_width::UnicodeWidthChar;

use super::app::{McpApp, McpDetailLayout, McpFocus};
use super::edit::McpEditFieldKind;
use crate::mcp::McpRegistration;
use crate::tui::common::{centered_rect, key_hints, render_border_beam};

const ACCENT: Color = Color::Cyan;
const ACTIVE_BORDER: Color = Color::Cyan;
const INACTIVE_BORDER: Color = Color::Rgb(60, 65, 75);
const SELECTION_BG: Color = Color::Rgb(40, 44, 52);
const LABEL: Color = Color::Rgb(150, 160, 190);
const SEPARATOR: Color = Color::Rgb(50, 55, 65);

pub(crate) fn draw_mcp(frame: &mut Frame<'_>, app: &mut McpApp) {
    let areas = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(8),
        Constraint::Length(1),
    ])
    .split(frame.area());

    render_header(frame, areas[0], app);
    if app.full_screen_detail {
        render_detail(frame, areas[1], app);
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
    if app.editor.is_some() {
        render_editor(frame, app);
    }
}

fn render_header(frame: &mut Frame<'_>, area: Rect, app: &McpApp) {
    render_border_beam(
        frame,
        area,
        app.marquee_offset,
        " MCP Registration Browser ",
        INACTIVE_BORDER,
        ACCENT,
    );
    let inner = Rect::new(
        area.x.saturating_add(2),
        area.y.saturating_add(1),
        area.width.saturating_sub(4),
        1,
    );
    let spans = if app.is_searching {
        vec![
            Span::styled(" MENA MCP ", Style::default().fg(ACCENT).bold()),
            Span::styled("│ Filter: ", Style::default().fg(LABEL)),
            Span::styled(
                format!("{}█", app.query),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(
                format!(" ({}/{})", app.visible.len(), app.registrations.len()),
                Style::default().fg(Color::DarkGray),
            ),
        ]
    } else if let Some(notice) = &app.notice {
        vec![
            Span::styled(" MENA MCP ", Style::default().fg(ACCENT).bold()),
            Span::styled("│ ", Style::default().fg(SEPARATOR)),
            Span::styled(
                notice.message.clone(),
                Style::default().fg(if notice.error {
                    Color::Red
                } else {
                    Color::Green
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
            Span::styled(" MENA MCP ", Style::default().fg(ACCENT).bold()),
            Span::styled("│ ", Style::default().fg(SEPARATOR)),
            Span::styled(probe, Style::default().fg(LABEL)),
        ]
    };
    frame.render_widget(Paragraph::new(Line::from(spans)), inner);
}

fn render_list(frame: &mut Frame<'_>, area: Rect, app: &McpApp) {
    let compact = area.width < 54;
    let rows = app.visible.iter().map(|catalog_index| {
        let registration = &app.registrations[*catalog_index];
        let state = registration_state(registration);
        if compact {
            Row::new(vec![
                Cell::from(format!("{}/{}", registration.scope, registration.name)),
                Cell::from(registration.provider.clone()),
                Cell::from(state),
            ])
        } else {
            Row::new(vec![
                Cell::from(registration.name.clone()),
                Cell::from(registration.provider.clone()),
                Cell::from(registration.scope.clone()),
                Cell::from(registration.transport.as_str()),
                Cell::from(state),
            ])
        }
    });
    let widths: Vec<Constraint> = if compact {
        vec![
            Constraint::Min(12),
            Constraint::Length(9),
            Constraint::Length(9),
        ]
    } else {
        vec![
            Constraint::Min(16),
            Constraint::Length(9),
            Constraint::Length(9),
            Constraint::Length(16),
            Constraint::Length(9),
        ]
    };
    let headers = if compact {
        vec!["SCOPE/NAME", "CLIENT", "STATE"]
    } else {
        vec!["NAME", "CLIENT", "SCOPE", "TRANSPORT", "STATE"]
    };
    let active = app.focus == McpFocus::List && !app.full_screen_detail;
    let title = format!(
        " {}Registrations ({}/{}) ",
        if active { "▸ " } else { "" },
        app.visible.len(),
        app.registrations.len()
    );
    let table = Table::new(rows, widths)
        .header(Row::new(headers).style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)))
        .row_highlight_style(
            Style::default()
                .bg(SELECTION_BG)
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ")
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(if active {
                    ACTIVE_BORDER
                } else {
                    INACTIVE_BORDER
                }))
                .title(title),
        );
    let mut state = TableState::default();
    state.select((!app.visible.is_empty()).then_some(app.selected_index));
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
        || " MCP Inspector ".to_owned(),
        |(_, selector, probing)| {
            format!(
                " {}MCP Inspector: {selector}{} ",
                if active { "▸ " } else { "" },
                if *probing { " [PROBING]" } else { "" }
            )
        },
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if app.full_screen_detail {
            Color::Yellow
        } else if active {
            ACTIVE_BORDER
        } else {
            INACTIVE_BORDER
        }))
        .title(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some((registration_index, _, probing)) = selected else {
        frame.render_widget(
            Paragraph::new("No MCP registration selected")
                .style(Style::default().fg(Color::DarkGray)),
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
        Paragraph::new(key_hints(&[
            ("o", "Open config"),
            ("e", "Edit basics"),
            ("p", "Probe"),
            ("↑/↓", "Move/scroll"),
            ("Tab", "Pane"),
            ("/", "Search"),
            ("Enter", "Fullscreen"),
            ("q/Esc", "Back"),
        ])),
        area,
    );
}

fn render_editor(frame: &mut Frame<'_>, app: &McpApp) {
    let editor = app.editor.as_ref().expect("checked MCP editor");
    let preferred_height = u16::try_from(editor.fields.len())
        .unwrap_or(u16::MAX)
        .saturating_add(if editor.error.is_some() { 9 } else { 7 });
    let area = centered_rect(frame.area(), 96, preferred_height);
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Yellow))
        .title(format!(" Edit MCP basics: {} ", editor.selector));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines = vec![
        Line::from(vec![
            Span::styled("Source: ", Style::default().fg(LABEL)),
            Span::styled(
                editor.source.display().to_string(),
                Style::default().fg(Color::Gray),
            ),
        ]),
        Line::default(),
    ];
    let value_width = usize::from(inner.width).saturating_sub(27).max(8);
    for (index, field) in editor.fields.iter().enumerate() {
        let selected = index == editor.selected;
        let marker = if selected { "▶" } else { " " };
        let dirty = if field.is_dirty() { "*" } else { " " };
        let value = editor_field_value(editor, index, value_width);
        let value_color = if selected {
            Color::White
        } else if field.is_dirty() {
            Color::Yellow
        } else {
            Color::Gray
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{marker}{dirty} {:<22} ", field.kind.label()),
                Style::default().fg(if selected { ACCENT } else { LABEL }),
            ),
            Span::styled(value, Style::default().fg(value_color)),
        ]));
    }
    lines.push(Line::default());
    if let Some(error) = &editor.error {
        lines.push(Line::from(Span::styled(
            format!("Error: {error}"),
            Style::default().fg(Color::Red),
        )));
        lines.push(Line::default());
    }
    lines.push(key_hints(&[
        ("↑/↓", "Field"),
        ("Enter", "Edit"),
        ("Space", "Toggle"),
        ("Ctrl+S", "Save"),
        ("Esc", "Cancel"),
    ]));
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

fn editor_field_value(editor: &super::edit::McpEditForm, index: usize, max_chars: usize) -> String {
    let field = &editor.fields[index];
    if field.kind == McpEditFieldKind::Enabled {
        return if field.value == "true" {
            "ON".to_owned()
        } else {
            "OFF".to_owned()
        };
    }
    let editing = editor.editing && editor.selected == index;
    let mut characters = field.value.chars().collect::<Vec<_>>();
    let cursor = if editing {
        field.value[..editor.cursor].chars().count()
    } else {
        0
    };
    if editing {
        characters.insert(cursor.min(characters.len()), '█');
    }
    if characters.len() <= max_chars {
        return characters.into_iter().collect();
    }
    let focus = if editing { cursor } else { 0 };
    let start = focus.saturating_sub(max_chars.saturating_sub(2) / 2);
    let start = start.min(characters.len().saturating_sub(max_chars));
    let mut visible = characters
        .into_iter()
        .skip(start)
        .take(max_chars)
        .collect::<String>();
    if start > 0 {
        visible = format!("…{}", visible.chars().skip(1).collect::<String>());
    }
    visible
}

const fn registration_state(registration: &McpRegistration) -> &'static str {
    if !registration.valid {
        "invalid"
    } else if registration.enabled {
        "enabled"
    } else {
        "disabled"
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
        Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
    } else if line == "Static registration metadata"
        || line.starts_with("Runtime metadata:")
        || line.starts_with("Runtime tools:")
        || line.starts_with("Runtime prompts:")
        || line.starts_with("Runtime resources:")
    {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
    } else if line.contains("Error:") || line.starts_with("Probe request failed:") {
        Style::default().fg(Color::Red)
    } else if line.contains("Warning:") {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Gray)
    }
}
