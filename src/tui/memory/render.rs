use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Cell, Clear, Paragraph, Row, Table, Wrap};

use super::app::*;
use crate::tui::common::{
    UI, app_header, badge, centered_rect, header_inner, key_hints, panel_block, panel_title,
    render_canvas, render_header_frame, render_keymap_popup, responsive_key_hints, scroll_meter,
    selection_style, table_header_style,
};

pub(crate) fn draw_memories(frame: &mut Frame<'_>, app: &mut MemoriesApp) {
    render_canvas(frame);
    let areas = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(8),
        Constraint::Length(1),
    ])
    .split(frame.area());

    render_header(frame, areas[0], app);

    if areas[1].width < 100 {
        let rows = Layout::vertical([Constraint::Percentage(46), Constraint::Percentage(54)])
            .split(areas[1]);
        render_list(frame, rows[0], app);
        render_preview(frame, rows[1], app);
    } else {
        let columns = Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(areas[1]);
        render_list(frame, columns[0], app);
        render_preview(frame, columns[1], app);
    }

    render_footer(frame, areas[2], app);
    if app.pending_delete.is_some() {
        render_delete_confirmation(frame, app);
    }
    if app.show_help {
        render_keymap_popup(frame, "Memories", MEMORY_HELP_KEYS);
    }
}

const MEMORY_HELP_KEYS: &[(&str, &str)] = &[
    ("↑/↓ j/k", "move through files or scroll the preview"),
    ("Tab / l", "switch between list and preview"),
    ("/", "filter by name, provider, scope, location"),
    ("o", "edit the selected file in $VISUAL/$EDITOR"),
    ("d", "delete the selected file (y confirms)"),
    ("1/2/3", "switch to Sessions / Skills / MCP"),
    ("?", "toggle this keymap"),
    ("q / Esc", "quit"),
];

fn render_header(frame: &mut Frame<'_>, area: Rect, app: &MemoriesApp) {
    render_header_frame(frame, area, " Memories ");
    let context = if app.is_searching {
        vec![
            badge("Filter", UI.amber),
            Span::styled("  Query: ", Style::default().fg(UI.muted)),
            Span::styled(
                format!("{}▌", app.query),
                Style::default().fg(UI.amber).add_modifier(Modifier::BOLD),
            ),
        ]
    } else if !app.query.is_empty() {
        vec![
            badge("Filtered", UI.amber),
            Span::styled(
                format!("  Query: {}", app.query),
                Style::default().fg(UI.amber),
            ),
            Span::styled(
                format!("  {}/{} loaded", app.visible.len(), app.files.len()),
                Style::default().fg(UI.muted),
            ),
        ]
    } else {
        vec![
            badge("All", UI.cyan),
            Span::styled(
                format!("  {} memory files", app.files.len()),
                Style::default().fg(UI.text_soft),
            ),
        ]
    };
    frame.render_widget(
        Paragraph::new(app_header("Memories", context)),
        header_inner(area),
    );
}

fn render_list(frame: &mut Frame<'_>, area: Rect, app: &MemoriesApp) {
    let mut rows = Vec::with_capacity(app.visible.len());
    let mut selected_row = None;
    for (visible_index, catalog_index) in app.visible.iter().enumerate() {
        let file = &app.files[*catalog_index];
        if visible_index == app.selected_index {
            selected_row = Some(rows.len());
        }
        let style = if visible_index == app.selected_index {
            selection_style()
        } else {
            Style::default().fg(UI.text).bg(UI.panel)
        };
        rows.push(
            Row::new(vec![
                Cell::from(Span::styled(
                    file.name.clone(),
                    if visible_index == app.selected_index {
                        Style::default().fg(UI.text).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(UI.text_soft)
                    },
                )),
                Cell::from(file.provider.clone()),
                Cell::from(file.scope.clone()),
                Cell::from(crate::view::format_bytes(file.size_bytes)),
            ])
            .style(style),
        );
    }
    let active = app.focus == MemoryFocus::List;
    let table = Table::new(
        rows,
        [
            Constraint::Min(20),
            Constraint::Length(10),
            Constraint::Length(9),
            Constraint::Length(10),
        ],
    )
    .header(Row::new(["Name", "Provider", "Scope", "Size"]).style(table_header_style()))
    .row_highlight_style(selection_style())
    .highlight_symbol("> ")
    .block(panel_block(
        panel_title(
            "Memory files",
            Some(format!(
                "{} visible · {} total",
                app.visible.len(),
                app.files.len()
            )),
            active,
        ),
        active,
    ));
    let mut state = ratatui::widgets::TableState::default();
    state.select(selected_row);
    frame.render_stateful_widget(table, area, &mut state);
}

fn render_preview(frame: &mut Frame<'_>, area: Rect, app: &mut MemoriesApp) {
    let active = app.focus == MemoryFocus::Preview;
    let title = panel_title(
        "Preview",
        Some(scroll_meter(app.preview_scroll, app.preview_max, 5)),
        active,
    );
    let block = panel_block(title, active);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some((_, content)) = app.preview.as_ref() else {
        frame.render_widget(Paragraph::new("No memory file selected"), inner);
        app.preview_max = 0;
        return;
    };
    let text = match content {
        Ok(content) => Text::from(content.clone()),
        Err(error) => Text::from(Span::styled(error.clone(), Style::default().fg(UI.danger))),
    };
    let paragraph = Paragraph::new(text).wrap(Wrap { trim: false });
    let total = paragraph.line_count(inner.width.max(1));
    app.preview_max = total.saturating_sub(usize::from(inner.height));
    app.preview_scroll = app.preview_scroll.min(app.preview_max);
    frame.render_widget(
        paragraph.scroll((u16::try_from(app.preview_scroll).unwrap_or(u16::MAX), 0)),
        inner,
    );
}

fn render_footer(frame: &mut Frame<'_>, area: Rect, app: &MemoriesApp) {
    let footer = match app.status.as_ref() {
        Some((message, is_error)) => Line::from(Span::styled(
            message.clone(),
            Style::default().fg(if *is_error { UI.danger } else { UI.success }),
        )),
        None => responsive_key_hints(
            area.width,
            &[
                ("/", "filter"),
                ("o", "edit"),
                ("d", "delete"),
                ("Tab", "pane"),
                ("?", "keys"),
                ("q", "quit"),
            ],
            &[
                ("/", "filter"),
                ("o", "edit"),
                ("d", "delete"),
                ("q", "quit"),
            ],
        ),
    };
    frame.render_widget(Paragraph::new(footer), area);
}

fn render_delete_confirmation(frame: &mut Frame<'_>, app: &MemoriesApp) {
    let Some(file) = app
        .pending_delete
        .and_then(|index| app.visible.get(index))
        .and_then(|index| app.files.get(*index))
    else {
        return;
    };
    let area = centered_rect(frame.area(), 84, 9);
    frame.render_widget(Clear, area);
    let block = panel_block(
        panel_title(
            "Delete memory file",
            Some("this cannot be undone".to_owned()),
            true,
        ),
        true,
    )
    .border_style(Style::default().fg(UI.danger));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let lines = vec![
        Line::from(vec![
            Span::styled("File:   ", Style::default().fg(UI.muted)),
            Span::styled(
                format!("{}:{}", file.provider, file.scope),
                Style::default().fg(UI.text).bold(),
            ),
        ]),
        Line::from(vec![
            Span::styled("Name:   ", Style::default().fg(UI.muted)),
            Span::styled(file.name.clone(), Style::default().fg(UI.text)),
        ]),
        Line::from(vec![
            Span::styled("Path:   ", Style::default().fg(UI.muted)),
            Span::styled(
                file.path.display().to_string(),
                Style::default().fg(UI.text_soft),
            ),
        ]),
        Line::default(),
        key_hints(&[("y", "delete permanently"), ("n/Esc", "cancel")]),
    ];
    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: true }),
        inner,
    );
}
