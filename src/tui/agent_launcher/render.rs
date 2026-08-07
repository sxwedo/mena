use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use super::AgentLauncherItem;
use crate::AgentKind;
use crate::tui::common::{ACCENT, MUTED};

pub(crate) fn draw_agent_selector(
    frame: &mut Frame<'_>,
    items: &[AgentLauncherItem],
    selected_index: usize,
) {
    let area = frame.area();
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(5),
        Constraint::Length(3),
    ])
    .split(area);

    let title_paragraph = Paragraph::new(Line::from(vec![
        Span::styled(
            "mena agent ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::raw("— Select Developer Agent for Current Directory"),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(ACCENT)),
    );
    frame.render_widget(title_paragraph, chunks[0]);

    let rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            let selected = idx == selected_index;
            let slug = item.kind.slug();
            let label = format!("{}", item.kind);

            let (status_text, status_color) = if item.installed {
                ("✓ in PATH", Color::Green)
            } else {
                ("✗ not in PATH", MUTED)
            };

            let session_info = if item.session_count > 0 {
                let title = item.latest_session_title.as_deref().unwrap_or("Untitled");
                format!("{} session(s) in cwd (latest: {title})", item.session_count)
            } else {
                "no saved sessions in cwd".to_owned()
            };

            let text_color = if item.installed { Color::Reset } else { MUTED };

            let row = Row::new(vec![
                Cell::from(format!("{slug} ({label})")).style(Style::default().fg(text_color)),
                Cell::from(status_text).style(Style::default().fg(status_color)),
                Cell::from(session_info).style(Style::default().fg(text_color)),
            ]);

            if selected {
                if item.installed {
                    row.style(
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    )
                } else {
                    row.style(
                        Style::default()
                            .fg(Color::Gray)
                            .add_modifier(Modifier::BOLD),
                    )
                }
            } else {
                row
            }
        })
        .collect();

    let widths = [
        Constraint::Percentage(35),
        Constraint::Percentage(20),
        Constraint::Percentage(45),
    ];

    let table = Table::new(rows, widths)
        .header(
            Row::new(vec!["AGENT", "STATUS", "CWD SESSIONS"])
                .style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Available Coding Agents "),
        );

    frame.render_widget(table, chunks[1]);

    let selected_installed = items.get(selected_index).is_none_or(|item| item.installed);

    let footer_text = if selected_installed {
        vec![Line::from(vec![
            Span::styled(
                "[Enter] ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::raw("Launch/Select  "),
            Span::styled(
                "[n] ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::raw("New Session  "),
            Span::styled(
                "[r] ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::raw("Resume Latest  "),
            Span::styled(
                "[Esc/q] ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::raw("Quit"),
        ])]
    } else {
        let homepage = items
            .get(selected_index)
            .map_or("", |item| item.kind.homepage_url());
        vec![Line::from(vec![
            Span::styled(
                "[Enter] ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("Open Homepage ({homepage})  ")),
            Span::styled(
                "[Esc/q] ",
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::raw("Quit"),
        ])]
    };

    let footer = Paragraph::new(footer_text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(MUTED)),
    );
    frame.render_widget(footer, chunks[2]);
}

pub(crate) fn draw_mode_selector<T>(
    frame: &mut Frame<'_>,
    kind: &AgentKind,
    options: &[(T, String)],
    selected_index: usize,
) {
    let area = frame.area();
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(5),
        Constraint::Length(3),
    ])
    .split(area);

    let title_paragraph = Paragraph::new(Line::from(vec![
        Span::styled(
            format!("mena agent {} ", kind.slug()),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::raw("— Choose Launch Mode for Current Directory"),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(ACCENT)),
    );
    frame.render_widget(title_paragraph, chunks[0]);

    let rows: Vec<Row> = options
        .iter()
        .enumerate()
        .map(|(idx, (_, label))| {
            let selected = idx == selected_index;
            let row = Row::new(vec![Cell::from(label.clone())]);
            if selected {
                row.style(
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                row
            }
        })
        .collect();

    let widths = [Constraint::Percentage(100)];
    let table = Table::new(rows, widths)
        .header(
            Row::new(vec!["LAUNCH MODE"])
                .style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Launch Options for {kind} ")),
        );

    frame.render_widget(table, chunks[1]);

    let footer_text = vec![Line::from(vec![
        Span::styled(
            "[Enter] ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::raw("Confirm  "),
        Span::styled(
            "[Esc/q] ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::raw("Cancel"),
    ])];

    let footer = Paragraph::new(footer_text).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(MUTED)),
    );
    frame.render_widget(footer, chunks[2]);
}
