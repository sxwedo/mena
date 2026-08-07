use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Clear, Paragraph, Row, Table};

use super::AgentLauncherItem;
use crate::AgentKind;
use crate::tui::common::{ACCENT, MUTED, centered_rect, render_border_beam};

// ── Design Tokens ─────────────────────────────────────────────────────────────

const COLOR_ACCENT: Color = Color::Cyan;
const COLOR_ACTIVE_BORDER: Color = Color::Cyan;
const COLOR_INACTIVE_BORDER: Color = Color::Rgb(60, 65, 75);
const COLOR_SELECTION_BG: Color = Color::Rgb(40, 44, 52);
const COLOR_LABEL_KEY: Color = Color::Rgb(150, 160, 190);
const COLOR_SEPARATOR: Color = Color::Rgb(50, 55, 65);

const fn agent_icon(kind: &AgentKind) -> &'static str {
    match kind {
        AgentKind::ClaudeCode => "🤖 ",
        AgentKind::Codex => "🧠 ",
        AgentKind::GeminiCli => "💎 ",
        AgentKind::OpenCode => "🔓 ",
        AgentKind::Pi => "🥧 ",
        AgentKind::OhMyPi => "⚡ ",
        AgentKind::Cursor => "💻 ",
        AgentKind::Goose => "🪶 ",
        AgentKind::Custom(_) => "🛠️  ",
    }
}

pub(crate) fn draw_agent_selector(
    frame: &mut Frame<'_>,
    items: &[AgentLauncherItem],
    selected_index: usize,
    tick: usize,
) {
    let area = frame.area();

    let chunks = Layout::vertical([
        Constraint::Length(3), // Header
        Constraint::Min(8),    // Table
        Constraint::Length(1), // Footer
    ])
    .split(area);
    // 1. Header
    render_border_beam(
        frame,
        chunks[0],
        tick,
        " Developer Agent Launcher ",
        COLOR_INACTIVE_BORDER,
        Color::Cyan,
    );

    let title_paragraph = Paragraph::new(Line::from(vec![
        Span::styled(
            " ⚡ MENA LAUNCHER ",
            Style::default()
                .fg(COLOR_ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("│ ", Style::default().fg(COLOR_SEPARATOR)),
        Span::styled(
            "Select & Launch Developer Agent in Current Directory",
            Style::default().fg(COLOR_LABEL_KEY),
        ),
    ]));
    frame.render_widget(title_paragraph, chunks[0]);

    // 2. Table Rows
    let rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            let selected = idx == selected_index;
            let slug = item.kind.slug();
            let label = format!("{}", item.kind);
            let icon = agent_icon(&item.kind);

            let cursor = if selected { "▶ " } else { "  " };

            let (status_text, status_color) = if item.installed {
                ("[✓ PATH]", Color::Green)
            } else {
                ("[✗ MISSING]", MUTED)
            };

            let session_info = if item.session_count > 0 {
                let title = item.latest_session_title.as_deref().unwrap_or("Untitled");
                format!("{} session(s) in cwd (latest: {title})", item.session_count)
            } else {
                "no saved sessions in cwd".to_owned()
            };

            let name_style = if selected {
                if item.installed {
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(Color::Gray)
                        .add_modifier(Modifier::BOLD)
                }
            } else if item.installed {
                Style::default().fg(Color::Reset)
            } else {
                Style::default().fg(MUTED)
            };

            let session_style = if selected {
                Style::default().fg(Color::Yellow)
            } else if item.session_count > 0 {
                Style::default().fg(Color::Reset)
            } else {
                Style::default().fg(MUTED)
            };

            let row_bg = if selected {
                Style::default().bg(COLOR_SELECTION_BG)
            } else {
                Style::default()
            };

            Row::new(vec![
                Cell::from(Span::styled(
                    format!("{cursor}{icon}{slug} ({label})"),
                    name_style,
                )),
                Cell::from(Span::styled(status_text, Style::default().fg(status_color))),
                Cell::from(Span::styled(session_info, session_style)),
            ])
            .style(row_bg)
        })
        .collect();

    let widths = [
        Constraint::Percentage(35),
        Constraint::Percentage(20),
        Constraint::Percentage(45),
    ];

    let table = Table::new(rows, widths)
        .header(
            Row::new(vec!["AGENT", "STATUS", "CWD SESSIONS"]).style(
                Style::default()
                    .fg(COLOR_ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(COLOR_ACTIVE_BORDER))
                .title(" Available Coding Agents "),
        );

    frame.render_widget(table, chunks[1]);

    // 3. Footer Pill Badges
    let selected_installed = items.get(selected_index).is_none_or(|item| item.installed);

    let footer_spans = if selected_installed {
        vec![
            Span::styled(
                " Enter ",
                Style::default()
                    .fg(Color::Black)
                    .bg(COLOR_ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Launch/Select ", Style::default().fg(Color::Gray)),
            Span::styled("│ ", Style::default().fg(COLOR_SEPARATOR)),
            Span::styled(
                " n ",
                Style::default()
                    .fg(Color::Black)
                    .bg(COLOR_ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" New Session ", Style::default().fg(Color::Gray)),
            Span::styled("│ ", Style::default().fg(COLOR_SEPARATOR)),
            Span::styled(
                " r ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Resume Latest ", Style::default().fg(Color::Gray)),
            Span::styled("│ ", Style::default().fg(COLOR_SEPARATOR)),
            Span::styled(
                " q/Esc ",
                Style::default()
                    .fg(Color::Black)
                    .bg(COLOR_ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Quit ", Style::default().fg(Color::Gray)),
        ]
    } else {
        let homepage = items
            .get(selected_index)
            .map_or("", |item| item.kind.homepage_url());
        let clean_url = if homepage.len() > 40 {
            format!("{}...", &homepage[..37])
        } else {
            homepage.to_string()
        };
        vec![
            Span::styled(
                " Enter ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" Open Homepage ({clean_url}) "),
                Style::default().fg(Color::Gray),
            ),
            Span::styled("│ ", Style::default().fg(COLOR_SEPARATOR)),
            Span::styled(
                " q/Esc ",
                Style::default()
                    .fg(Color::Black)
                    .bg(COLOR_ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" Quit ", Style::default().fg(Color::Gray)),
        ]
    };

    let footer = Paragraph::new(Line::from(footer_spans));
    frame.render_widget(footer, chunks[2]);
}

pub(crate) fn draw_mode_selector<T>(
    frame: &mut Frame<'_>,
    kind: &AgentKind,
    options: &[(T, String)],
    selected_index: usize,
    tick: usize,
) {
    let area = frame.area();

    // Render as a centered floating modal card (not full-screen)
    let popup_area = centered_rect(area, 66, 12);
    frame.render_widget(Clear, popup_area);

    render_border_beam(
        frame,
        popup_area,
        tick,
        &format!(" Select Session Mode: {kind} "),
        COLOR_INACTIVE_BORDER,
        Color::Yellow,
    );

    let chunks = Layout::vertical([
        Constraint::Length(3), // Header
        Constraint::Min(4),    // Options
        Constraint::Length(1), // Footer
    ])
    .split(popup_area);

    let icon = agent_icon(kind);

    let title_paragraph = Paragraph::new(Line::from(vec![Span::styled(
        format!(" {icon} Launch Options: {kind} "),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Yellow))
            .title(" Select Session Mode "),
    );
    frame.render_widget(title_paragraph, chunks[0]);

    let rows: Vec<Row> = options
        .iter()
        .enumerate()
        .map(|(idx, (_, label))| {
            let selected = idx == selected_index;
            let cursor = if selected { "▶ " } else { "  " };

            let name_style = if selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Reset)
            };

            let row_bg = if selected {
                Style::default().bg(COLOR_SELECTION_BG)
            } else {
                Style::default()
            };

            Row::new(vec![Cell::from(Span::styled(
                format!("{cursor}{label}"),
                name_style,
            ))])
            .style(row_bg)
        })
        .collect();

    let widths = [Constraint::Percentage(100)];
    let table = Table::new(rows, widths).block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(COLOR_INACTIVE_BORDER)),
    );

    frame.render_widget(table, chunks[1]);

    let footer_spans = vec![
        Span::styled(
            " Enter ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Confirm ", Style::default().fg(Color::Gray)),
        Span::styled("│ ", Style::default().fg(COLOR_SEPARATOR)),
        Span::styled(
            " q/Esc ",
            Style::default()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" Cancel ", Style::default().fg(Color::Gray)),
    ];

    let footer = Paragraph::new(Line::from(footer_spans));
    frame.render_widget(footer, chunks[2]);
}
