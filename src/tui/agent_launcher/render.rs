use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Clear, Paragraph, Row, Table};

use super::AgentLauncherItem;
use crate::AgentKind;
use crate::tui::common::{
    UI, app_header, badge, centered_rect, header_inner, panel_block, panel_title, render_canvas,
    render_header_frame, responsive_key_hints, selection_style, table_header_style,
};

// Provider colors stay recognizable while sharing the calm, low-saturation palette.
const COLOR_SKY: Color = UI.cyan;
const COLOR_CLAUDE: Color = Color::Rgb(194, 141, 119);
const COLOR_OPENAI: Color = UI.success;
const COLOR_GEMINI: Color = UI.signal;
const COLOR_PI: Color = UI.violet;
const COLOR_CURSOR: Color = Color::Rgb(139, 165, 196);
const COLOR_GOOSE: Color = UI.amber;
const COLOR_CUSTOM: Color = UI.text_soft;

/// Returns `(symbol, brand_color)` for each agent — no emoji, pure Unicode
/// geometric shapes that evoke each brand's visual identity.
const fn agent_icon(kind: &AgentKind) -> (&'static str, Color) {
    match kind {
        // ✳ sunburst — evokes Anthropic Claude's logo
        AgentKind::ClaudeCode => ("✳", COLOR_CLAUDE),
        // ❂ — evokes OpenAI's hexagonal flower bloom
        AgentKind::Codex => ("❂", COLOR_OPENAI),
        // ✦ 4-point star — matches Google Gemini's sparkle
        AgentKind::GeminiCli => ("✦", COLOR_GEMINI),
        // ▣ filled checked box — "open code" concept
        AgentKind::OpenCode => ("▣", COLOR_SKY),
        // π — the actual Greek letter IS the Pi brand
        AgentKind::Pi => ("π", COLOR_PI),
        // ϟ lightning sigil — Oh My Pi's energy identity without emoji rendering.
        AgentKind::OhMyPi => ("ϟ", UI.amber),
        // ❯ prompt cursor — matches Cursor editor's brand
        AgentKind::Cursor => ("❯", COLOR_CURSOR),
        // ◈ diamond — Goose's geometric identity
        AgentKind::Goose => ("◈", COLOR_GOOSE),
        // ◇ neutral operator-defined target.
        AgentKind::Custom(_) => ("◇", COLOR_CUSTOM),
    }
}

pub(crate) fn draw_agent_selector(
    frame: &mut Frame<'_>,
    items: &[AgentLauncherItem],
    selected_index: usize,
    _tick: usize,
) {
    let area = frame.area();
    render_canvas(frame);

    let chunks = Layout::vertical([
        Constraint::Length(3), // Header
        Constraint::Min(8),    // Table
        Constraint::Length(1), // Footer
    ])
    .split(area);
    render_header_frame(frame, chunks[0], " Agent launcher ");
    let installed = items.iter().filter(|item| item.installed).count();
    let title_paragraph = Paragraph::new(app_header(
        "Agents",
        vec![
            badge("Local", UI.cyan),
            Span::styled(
                format!("  {installed} of {} available", items.len()),
                Style::default().fg(UI.text_soft),
            ),
        ],
    ));
    frame.render_widget(title_paragraph, header_inner(chunks[0]));

    let compact = chunks[1].width < 88;
    let rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            let selected = idx == selected_index;
            let slug = item.kind.slug();
            let label = format!("{}", item.kind);
            let (icon, icon_color) = agent_icon(&item.kind);

            let cursor = if selected { "> " } else { "  " };

            let (status_text, status_color) = if item.installed {
                ("Ready", UI.success)
            } else {
                ("Missing", UI.muted)
            };

            let session_info = if item.session_count > 0 {
                let title = item.latest_session_title.as_deref().unwrap_or("Untitled");
                format!("{} session(s) here · latest: {title}", item.session_count)
            } else {
                "No saved sessions in this directory".to_owned()
            };

            let name_style = if selected {
                if item.installed {
                    Style::default().fg(UI.text).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(UI.text_soft)
                        .add_modifier(Modifier::BOLD)
                }
            } else if item.installed {
                Style::default().fg(UI.text)
            } else {
                Style::default().fg(UI.muted)
            };

            let session_style = if item.session_count > 0 {
                Style::default().fg(UI.text_soft)
            } else {
                Style::default().fg(UI.muted)
            };

            let row_style = if selected {
                selection_style()
            } else {
                Style::default().fg(UI.text).bg(UI.panel)
            };

            let mut cells = vec![
                Cell::from(Line::from(vec![
                    Span::styled(
                        cursor,
                        Style::default().fg(icon_color).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!("{icon} {slug} ({label})"), name_style),
                ])),
                Cell::from(badge(status_text, status_color)),
            ];
            if !compact {
                cells.push(Cell::from(Span::styled(session_info, session_style)));
            }
            Row::new(cells).style(row_style)
        })
        .collect();

    let widths = if compact {
        vec![Constraint::Min(24), Constraint::Length(10)]
    } else {
        vec![
            Constraint::Percentage(35),
            Constraint::Length(10),
            Constraint::Min(30),
        ]
    };
    let headers = if compact {
        vec!["Agent", "Status"]
    } else {
        vec!["Agent", "Status", "Current directory / sessions"]
    };

    let table = Table::new(rows, widths)
        .header(Row::new(headers).style(table_header_style()))
        .block(panel_block(
            panel_title(
                "Available agents",
                Some(format!("{} total", items.len())),
                true,
            ),
            true,
        ));

    frame.render_widget(table, chunks[1]);

    let selected_installed = items.get(selected_index).is_none_or(|item| item.installed);

    let footer = if selected_installed {
        responsive_key_hints(
            chunks[2].width,
            &[
                ("Enter", "select"),
                ("n", "new"),
                ("r", "resume latest"),
                ("↑/↓", "navigate"),
                ("q", "quit"),
            ],
            &[
                ("Enter", "select"),
                ("n", "new"),
                ("r", "resume"),
                ("q", "quit"),
            ],
        )
    } else {
        let homepage = items
            .get(selected_index)
            .map_or("", |item| item.kind.homepage_url());
        let clean_url = if homepage.len() > 40 {
            format!("{}...", &homepage[..37])
        } else {
            homepage.to_string()
        };
        Line::from(vec![
            Span::styled("[Enter]", Style::default().fg(UI.signal).bold()),
            Span::styled(
                format!(" homepage {clean_url}"),
                Style::default().fg(UI.text_soft),
            ),
            Span::styled("  │  ", Style::default().fg(UI.grid)),
            Span::styled("[q]", Style::default().fg(UI.signal).bold()),
            Span::styled(" quit", Style::default().fg(UI.text_soft)),
        ])
    };

    frame.render_widget(Paragraph::new(footer), chunks[2]);
}

pub(crate) fn draw_mode_selector<T>(
    frame: &mut Frame<'_>,
    kind: &AgentKind,
    options: &[(T, String)],
    selected_index: usize,
    _tick: usize,
) {
    let area = frame.area();
    render_canvas(frame);

    // Render as a centered floating modal card (not full-screen)
    let popup_w = u16::min(85, area.width.saturating_sub(4));
    let popup_h = u16::min(
        u16::try_from(options.len() + 7).unwrap_or(20),
        area.height.saturating_sub(4),
    );
    let popup_area = centered_rect(area, popup_w, popup_h);
    frame.render_widget(Clear, popup_area);

    render_header_frame(frame, popup_area, &format!(" Launch {kind} "));

    let chunks = Layout::vertical([
        Constraint::Length(3), // Header
        Constraint::Min(4),    // Options
        Constraint::Length(1), // Footer
    ])
    .split(popup_area);

    let (icon, icon_color) = agent_icon(kind);

    let title_paragraph = Paragraph::new(Line::from(vec![
        Span::styled(
            format!(" {icon} "),
            Style::default().fg(icon_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("Choose how to start {kind}"),
            Style::default().fg(UI.text).add_modifier(Modifier::BOLD),
        ),
    ]))
    .block(panel_block(panel_title("Launch mode", None, true), true));
    frame.render_widget(title_paragraph, chunks[0]);

    let rows: Vec<Row> = options
        .iter()
        .enumerate()
        .map(|(idx, (_, label))| {
            let selected = idx == selected_index;
            let cursor = if selected { "> " } else { "  " };

            let name_style = if selected {
                Style::default().fg(UI.text).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(UI.text_soft)
            };

            let row_bg = if selected {
                selection_style()
            } else {
                Style::default().bg(UI.panel).fg(UI.text)
            };

            Row::new(vec![Cell::from(Span::styled(
                format!("{cursor}{label}"),
                name_style,
            ))])
            .style(row_bg)
        })
        .collect();

    let widths = [Constraint::Percentage(100)];
    let table = Table::new(rows, widths).block(panel_block(
        panel_title(
            "Options",
            Some(format!("{} available", options.len())),
            false,
        ),
        false,
    ));

    frame.render_widget(table, chunks[1]);

    let footer = Paragraph::new(responsive_key_hints(
        chunks[2].width,
        &[
            ("Enter", "confirm"),
            ("↑/↓", "navigate"),
            ("q/Esc", "cancel"),
        ],
        &[("Enter", "confirm"), ("q", "cancel")],
    ));
    frame.render_widget(footer, chunks[2]);
}
