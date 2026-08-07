use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Paragraph, Row, Table, TableState, Wrap};

use super::app::*;

// ── Design Tokens ─────────────────────────────────────────────────────────────

const COLOR_ACCENT: Color = Color::Cyan;
const COLOR_ACTIVE_BORDER: Color = Color::Cyan;
const COLOR_INACTIVE_BORDER: Color = Color::Rgb(60, 65, 75);
const COLOR_SELECTION_BG: Color = Color::Rgb(40, 44, 52);
const COLOR_LABEL_KEY: Color = Color::Rgb(150, 160, 190);
const COLOR_DIR: Color = Color::Rgb(120, 180, 240);
const COLOR_SYMLINK: Color = Color::Rgb(240, 200, 100);
const COLOR_SEPARATOR: Color = Color::Rgb(50, 55, 65);

pub(crate) fn draw_skills(frame: &mut Frame, app: &SkillsApp) {
    let area = frame.area();

    let chunks = Layout::vertical([
        Constraint::Length(3), // Header
        Constraint::Min(10),   // Body
        Constraint::Length(1), // Footer
    ])
    .split(area);

    // 1. Header
    let count_info = app
        .visible_rows
        .iter()
        .filter(|r| matches!(r, SkillRow::Skill { .. }))
        .count();

    let header_spans = if app.is_searching {
        vec![
            Span::styled(
                " ⚡ MENA SKILLS ",
                Style::default()
                    .fg(COLOR_ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("│ ", Style::default().fg(COLOR_SEPARATOR)),
            Span::styled("Search: ", Style::default().fg(Color::Yellow)),
            Span::styled(
                format!("{}_", app.search_query),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]
    } else if !app.search_query.is_empty() {
        vec![
            Span::styled(
                " ⚡ MENA SKILLS ",
                Style::default()
                    .fg(COLOR_ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("│ ", Style::default().fg(COLOR_SEPARATOR)),
            Span::styled("Filter: ", Style::default().fg(COLOR_LABEL_KEY)),
            Span::styled(
                format!("\"{}\"", app.search_query),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(
                format!(" ({count_info}/{})", app.skills.len()),
                Style::default().fg(Color::DarkGray),
            ),
        ]
    } else {
        vec![
            Span::styled(
                " ⚡ MENA SKILLS ",
                Style::default()
                    .fg(COLOR_ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("│ ", Style::default().fg(COLOR_SEPARATOR)),
            Span::styled(
                format!("{count_info} skills loaded"),
                Style::default().fg(COLOR_LABEL_KEY),
            ),
        ]
    };

    let header = Paragraph::new(Line::from(header_spans)).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(COLOR_ACTIVE_BORDER))
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
    let symlink_status = if app.show_symlinks { "on" } else { "off" };
    let footer_spans = vec![
        Span::styled(
            " Space/→ ",
            Style::default()
                .fg(Color::Black)
                .bg(COLOR_ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Expand ", Style::default().fg(Color::Gray)),
        Span::styled("│ ", Style::default().fg(COLOR_SEPARATOR)),
        Span::styled(
            " ← ",
            Style::default()
                .fg(Color::Black)
                .bg(COLOR_ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Collapse ", Style::default().fg(Color::Gray)),
        Span::styled("│ ", Style::default().fg(COLOR_SEPARATOR)),
        Span::styled(
            " Tab ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Switch Pane ", Style::default().fg(Color::Gray)),
        Span::styled("│ ", Style::default().fg(COLOR_SEPARATOR)),
        Span::styled(
            " ↑/↓ ",
            Style::default()
                .fg(Color::Black)
                .bg(COLOR_ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Move ", Style::default().fg(Color::Gray)),
        Span::styled("│ ", Style::default().fg(COLOR_SEPARATOR)),
        Span::styled(
            " s ",
            Style::default()
                .fg(Color::Black)
                .bg(COLOR_SYMLINK)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" Symlinks ({symlink_status}) "),
            Style::default().fg(Color::Gray),
        ),
        Span::styled("│ ", Style::default().fg(COLOR_SEPARATOR)),
        Span::styled(
            " / ",
            Style::default()
                .fg(Color::Black)
                .bg(COLOR_ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Search ", Style::default().fg(Color::Gray)),
        Span::styled("│ ", Style::default().fg(COLOR_SEPARATOR)),
        Span::styled(
            " o ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Open ", Style::default().fg(Color::Gray)),
        Span::styled("│ ", Style::default().fg(COLOR_SEPARATOR)),
        Span::styled(
            " q/Esc ",
            Style::default()
                .fg(Color::Black)
                .bg(COLOR_ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Quit ", Style::default().fg(Color::Gray)),
    ];
    let footer = Paragraph::new(Line::from(footer_spans));
    frame.render_widget(footer, chunks[2]);
}

#[allow(clippy::too_many_lines)]
fn render_skill_list(frame: &mut Frame, area: Rect, app: &SkillsApp) {
    let mut rows: Vec<Row> = Vec::new();
    let visible = &app.visible_rows;

    for (list_idx, row) in visible.iter().enumerate() {
        let is_selected = list_idx == app.selected_index;
        let row_bg = if is_selected {
            Style::default().bg(COLOR_SELECTION_BG)
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
                    if *expanded { "▾ " } else { "▸ " }
                } else {
                    "  "
                };

                let cursor = if is_selected { "▶ " } else { "  " };
                let skill_icon = if skill.is_symlink { "🔗 " } else { "📄 " };
                let name_str = format!("{cursor}{expand_icon}{skill_icon}{}", skill.name);
                let name_style = if is_selected {
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Reset)
                };

                let desc_raw = skill.description.as_deref().unwrap_or("-");
                let desc_clean = desc_raw.split('\n').next().unwrap_or("-").trim();
                let desc_display = if desc_clean.chars().count() > 36 {
                    let truncated: String = desc_clean.chars().take(33).collect();
                    format!("{truncated}...")
                } else {
                    desc_clean.to_string()
                };

                let desc_style = if is_selected {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default().fg(Color::DarkGray)
                };

                let (type_str, type_color) = if skill.is_symlink {
                    ("link", COLOR_SYMLINK)
                } else if *has_children {
                    ("dir", COLOR_DIR)
                } else {
                    ("md", Color::DarkGray)
                };

                rows.push(
                    Row::new(vec![
                        Cell::from(Span::styled(name_str, name_style)),
                        Cell::from(Span::styled(desc_display, desc_style)),
                        Cell::from(Span::styled(
                            format!("[{type_str}]"),
                            Style::default().fg(type_color),
                        )),
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
                let indent: String = "  ".repeat(depth.saturating_sub(1));
                let connector = if *is_last { "└─ " } else { "├─ " };

                let expand_icon = if *is_dir {
                    if *expanded { "▾ " } else { "▸ " }
                } else {
                    ""
                };

                let item_icon = if *is_dir { "📁 " } else { "📄 " };
                let name_str = format!("{indent}{connector}{expand_icon}{item_icon}{name}");

                let name_style = if is_selected {
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD)
                } else if *is_dir {
                    Style::default().fg(COLOR_DIR)
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
                let type_color = if *is_dir { COLOR_DIR } else { Color::DarkGray };

                rows.push(
                    Row::new(vec![
                        Cell::from(Span::styled(name_str, name_style)),
                        Cell::from(""),
                        Cell::from(Span::styled(
                            format!("[{type_str}]"),
                            Style::default().fg(type_color),
                        )),
                    ])
                    .style(row_bg),
                );
            }
        }
    }

    let is_active_focus = app.focus == SkillFocus::List && !app.full_screen_preview;
    let (border_color, title_text) = if is_active_focus {
        (COLOR_ACTIVE_BORDER, " ▸ Skills Roster ")
    } else {
        (COLOR_INACTIVE_BORDER, " Skills Roster ")
    };

    let table = Table::new(
        rows,
        [
            Constraint::Min(22),   // NAME (with tree prefix)
            Constraint::Min(28),   // DESCRIPTION (wider)
            Constraint::Length(8), // TYPE (last, narrow)
        ],
    )
    .header(
        Row::new(vec!["NAME", "DESCRIPTION", "TYPE"]).style(
            Style::default()
                .fg(COLOR_ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(title_text),
    );

    let mut state = TableState::default();
    if !app.visible_rows.is_empty() {
        state.select(Some(app.selected_index));
    }

    frame.render_stateful_widget(table, area, &mut state);
}

#[allow(clippy::too_many_lines)]
fn render_skill_preview(frame: &mut Frame, area: Rect, app: &SkillsApp) {
    let is_active_focus = app.focus == SkillFocus::Detail && !app.full_screen_preview;

    let Some(detail) = &app.current_detail else {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(COLOR_INACTIVE_BORDER))
            .title(" Skill Inspector ");
        let inner_area = block.inner(area);
        frame.render_widget(block, area);

        let empty_p = Paragraph::new("No skill selected or failed to load content")
            .style(Style::default().fg(Color::DarkGray));
        frame.render_widget(empty_p, inner_area);
        return;
    };

    let skill = &detail.skill;

    let border_color = if app.full_screen_preview {
        Color::Yellow
    } else if is_active_focus {
        COLOR_ACTIVE_BORDER
    } else {
        COLOR_INACTIVE_BORDER
    };

    let mode_label = if app.full_screen_preview {
        "[FULLSCREEN]"
    } else if is_active_focus {
        "[ACTIVE]"
    } else {
        ""
    };

    let title_text = format!(" ▸ Skill Inspector: {} {mode_label} ", skill.name);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .title(title_text);

    let inner_area = block.inner(area);
    frame.render_widget(block, area);

    // Separator line
    let separator_str = "─".repeat(usize::from(inner_area.width));

    let mut lines = vec![
        Line::from(vec![
            Span::styled("Name:        ", Style::default().fg(COLOR_LABEL_KEY)),
            Span::styled(
                &skill.name,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Provider:    ", Style::default().fg(COLOR_LABEL_KEY)),
            Span::styled(&skill.provider, Style::default().fg(Color::Yellow)),
            Span::raw("   "),
            Span::styled("Scope: ", Style::default().fg(COLOR_LABEL_KEY)),
            Span::styled(&skill.scope, Style::default().fg(COLOR_ACCENT)),
            Span::raw("   "),
            Span::styled("Type: ", Style::default().fg(COLOR_LABEL_KEY)),
            Span::styled(
                if skill.is_symlink { "symlink" } else { "file" },
                Style::default().fg(if skill.is_symlink {
                    COLOR_SYMLINK
                } else {
                    Color::DarkGray
                }),
            ),
            Span::raw("   "),
            Span::styled("Status: ", Style::default().fg(COLOR_LABEL_KEY)),
            Span::styled(
                if skill.valid {
                    "✓ valid"
                } else {
                    "✗ invalid"
                },
                Style::default().fg(if skill.valid {
                    Color::Green
                } else {
                    Color::Red
                }),
            ),
        ]),
        Line::from(vec![
            Span::styled("Location:    ", Style::default().fg(COLOR_LABEL_KEY)),
            Span::styled(&skill.location, Style::default().fg(COLOR_ACCENT)),
        ]),
        Line::from(vec![
            Span::styled("Path:        ", Style::default().fg(COLOR_LABEL_KEY)),
            Span::styled(
                skill.path.display().to_string(),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
    ];

    if !skill.triggers.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("Triggers:    ", Style::default().fg(COLOR_LABEL_KEY)),
            Span::styled(
                skill.triggers.join(", "),
                Style::default().fg(Color::LightGreen),
            ),
        ]));
    }

    if let Some(desc) = &skill.description {
        lines.push(Line::from(vec![
            Span::styled("Description: ", Style::default().fg(COLOR_LABEL_KEY)),
            Span::styled(desc, Style::default().fg(Color::Gray)),
        ]));
    }

    lines.push(Line::from(Span::styled(
        &separator_str,
        Style::default().fg(COLOR_SEPARATOR),
    )));

    // Markdown content parsing & styling
    let mut in_code_block = false;

    for content_line in detail.content.lines() {
        if content_line.trim_start().starts_with("```") {
            in_code_block = !in_code_block;
            lines.push(Line::from(Span::styled(
                content_line,
                Style::default().fg(Color::DarkGray),
            )));
            continue;
        }

        if in_code_block {
            lines.push(Line::from(vec![
                Span::styled("│ ", Style::default().fg(COLOR_SEPARATOR)),
                Span::styled(content_line, Style::default().fg(Color::Rgb(180, 190, 200))),
            ]));
        } else if let Some(rest) = content_line.strip_prefix("# ") {
            lines.push(Line::from(vec![
                Span::styled("█ ", Style::default().fg(COLOR_ACCENT)),
                Span::styled(
                    rest,
                    Style::default()
                        .fg(COLOR_ACCENT)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        } else if let Some(rest) = content_line.strip_prefix("## ") {
            lines.push(Line::from(vec![
                Span::styled("▌ ", Style::default().fg(Color::Yellow)),
                Span::styled(
                    rest,
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        } else if let Some(rest) = content_line.strip_prefix("### ") {
            lines.push(Line::from(vec![
                Span::styled("▌ ", Style::default().fg(COLOR_LABEL_KEY)),
                Span::styled(
                    rest,
                    Style::default()
                        .fg(COLOR_LABEL_KEY)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
        } else if content_line.starts_with("---") {
            lines.push(Line::from(Span::styled(
                &separator_str,
                Style::default().fg(COLOR_SEPARATOR),
            )));
        } else if let Some(rest) = content_line
            .strip_prefix("- ")
            .or_else(|| content_line.strip_prefix("* "))
        {
            lines.push(Line::from(vec![
                Span::styled("  • ", Style::default().fg(COLOR_ACCENT)),
                Span::styled(rest, Style::default().fg(Color::Reset)),
            ]));
        } else {
            lines.push(Line::from(content_line.to_string()));
        }
    }

    let total_lines = u16::try_from(lines.len()).unwrap_or(u16::MAX);
    let visible_height = inner_area.height;
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll = app.preview_scroll.min(max_scroll);

    let progress_percent = if max_scroll == 0 {
        100
    } else {
        usize::from(scroll) * 100 / usize::from(max_scroll)
    };

    // Subtitle scroll indicator
    let scroll_info = format!(
        " [Scroll: {progress_percent}% | Line {}/{total_lines}] ",
        scroll + 1
    );
    let info_paragraph = Paragraph::new(Line::from(Span::styled(
        scroll_info,
        Style::default().fg(Color::DarkGray),
    )));

    let paragraph = Paragraph::new(Text::from(lines))
        .scroll((scroll, 0))
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, inner_area);

    // Render scroll info at top-right of inner area if space allows
    if inner_area.width > 35 && inner_area.height > 2 {
        let info_rect = Rect::new(
            inner_area.x + inner_area.width.saturating_sub(32),
            inner_area.y,
            30,
            1,
        );
        frame.render_widget(info_paragraph, info_rect);
    }
}
