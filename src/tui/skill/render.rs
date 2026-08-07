use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap};

use super::app::*;

pub(crate) fn draw_skills(frame: &mut Frame, app: &SkillsApp) {
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
