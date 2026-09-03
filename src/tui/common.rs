//! Shared terminal primitives and styling helpers used across TUI modules.

use std::io::{self, Write};

use anyhow::{Context, Result};
use crossterm::event::{KeyEvent, KeyEventKind};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders};
use ratatui::{DefaultTerminal, layout::Rect};

use crate::settings::{ConfigColor, SessionDetailColorSettings};

// ── Calm Console design system ────────────────────────────────────────────────

/// Shared low-saturation colors for mena's keyboard-first terminal interface.
///
/// Focus, information, success, caution, and danger each have one quiet color.
/// Large surfaces stay neutral so the interface remains comfortable during
/// long sessions and continues to read clearly in an 80-column terminal.
#[derive(Debug, Clone, Copy)]
pub(crate) struct UiPalette {
    pub canvas: Color,
    pub panel: Color,
    pub panel_alt: Color,
    pub selection: Color,
    pub grid: Color,
    pub border: Color,
    pub signal: Color,
    pub success: Color,
    pub cyan: Color,
    pub amber: Color,
    pub danger: Color,
    pub violet: Color,
    pub text: Color,
    pub text_soft: Color,
    pub muted: Color,
}

pub(crate) const UI: UiPalette = UiPalette {
    canvas: Color::Rgb(17, 20, 24),
    panel: Color::Rgb(23, 27, 33),
    panel_alt: Color::Rgb(30, 36, 44),
    selection: Color::Rgb(37, 50, 68),
    grid: Color::Rgb(43, 50, 60),
    border: Color::Rgb(52, 61, 73),
    signal: Color::Rgb(124, 167, 217),
    success: Color::Rgb(134, 185, 140),
    cyan: Color::Rgb(121, 184, 199),
    amber: Color::Rgb(211, 170, 110),
    danger: Color::Rgb(217, 123, 132),
    violet: Color::Rgb(169, 155, 203),
    text: Color::Rgb(225, 230, 235),
    text_soft: Color::Rgb(168, 176, 186),
    muted: Color::Rgb(115, 125, 137),
};

pub(crate) fn render_canvas(frame: &mut ratatui::Frame<'_>) {
    let area = frame.area();
    frame.render_widget(
        Block::new().style(Style::default().bg(UI.canvas).fg(UI.text)),
        area,
    );
}

pub(crate) fn app_header(section: &'static str, context: Vec<Span<'static>>) -> Line<'static> {
    let mut spans = vec![
        Span::styled(
            " mena ",
            Style::default().fg(UI.text).add_modifier(Modifier::BOLD),
        ),
        Span::styled("· ", Style::default().fg(UI.border)),
        Span::styled(
            section,
            Style::default().fg(UI.signal).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
    ];
    spans.extend(context);
    Line::from(spans)
}

pub(crate) fn panel_title(
    label: impl Into<String>,
    meta: Option<String>,
    active: bool,
) -> Line<'static> {
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(
            label.into(),
            Style::default()
                .fg(if active { UI.text } else { UI.text_soft })
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(meta) = meta {
        spans.extend([
            Span::styled("  ", Style::default().fg(UI.border)),
            Span::styled(meta, Style::default().fg(UI.muted)),
        ]);
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
}

pub(crate) fn panel_block(title: Line<'_>, active: bool) -> Block<'_> {
    Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if active { UI.signal } else { UI.border }))
        .style(Style::default().bg(UI.panel).fg(UI.text))
        .title(title)
}

pub(crate) fn selection_style() -> Style {
    Style::default()
        .bg(UI.selection)
        .fg(UI.text)
        .add_modifier(Modifier::BOLD)
}

pub(crate) fn table_header_style() -> Style {
    Style::default()
        .fg(UI.text_soft)
        .bg(UI.panel_alt)
        .add_modifier(Modifier::BOLD)
}

pub(crate) fn badge(label: impl Into<String>, color: Color) -> Span<'static> {
    Span::styled(
        format!("● {}", label.into()),
        Style::default().fg(color).add_modifier(Modifier::BOLD),
    )
}

pub(crate) fn scroll_meter(position: usize, max: usize, cells: usize) -> String {
    let percent = position.saturating_mul(100).checked_div(max).unwrap_or(100);
    let filled = if max == 0 {
        cells
    } else {
        position
            .saturating_mul(cells)
            .checked_div(max)
            .unwrap_or(cells)
    }
    .min(cells);
    format!(
        "[{}{}] {percent:>3}%",
        "■".repeat(filled),
        "·".repeat(cells.saturating_sub(filled))
    )
}

pub(crate) const fn header_inner(area: Rect) -> Rect {
    Rect::new(
        area.x.saturating_add(2),
        area.y.saturating_add(1),
        area.width.saturating_sub(4),
        area.height.saturating_sub(2),
    )
}

// ── Color mapping ──────────────────────────────────────────────────────────────

pub(super) const fn configured_color(color: ConfigColor) -> Color {
    match color {
        ConfigColor::Reset => Color::Reset,
        ConfigColor::Black => Color::Black,
        ConfigColor::Red => Color::Red,
        ConfigColor::Green => Color::Green,
        ConfigColor::Yellow => Color::Yellow,
        ConfigColor::Blue => Color::Blue,
        ConfigColor::Magenta => Color::Magenta,
        ConfigColor::Cyan => Color::Cyan,
        ConfigColor::Gray => Color::Gray,
        ConfigColor::DarkGray => Color::DarkGray,
        ConfigColor::LightRed => Color::LightRed,
        ConfigColor::LightGreen => Color::LightGreen,
        ConfigColor::LightYellow => Color::LightYellow,
        ConfigColor::LightBlue => Color::LightBlue,
        ConfigColor::LightMagenta => Color::LightMagenta,
        ConfigColor::LightCyan => Color::LightCyan,
        ConfigColor::White => Color::White,
        ConfigColor::Indexed(index) => Color::Indexed(index),
        ConfigColor::Rgb(red, green, blue) => Color::Rgb(red, green, blue),
    }
}

// ── Session detail theme ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub(crate) struct SessionDetailTheme {
    pub border: Color,
    pub popup_title: Color,
    pub metadata_key: Color,
    pub metadata_value: Color,
    pub conversation_header: Color,
    pub empty_text: Color,
    pub status_success: Color,
    pub status_error: Color,
    pub footer_key: Color,
    pub footer_text: Color,
    pub footer_separator: Color,
    pub user_header: Color,
    pub user_content: Color,
    pub assistant_header: Color,
    pub assistant_content: Color,
    pub skill_header: Color,
    pub skill_content: Color,
    pub tool_call_header: Color,
    pub tool_call_content: Color,
    pub tool_result_header: Color,
    pub tool_result_content: Color,
    pub system_header: Color,
    pub system_content: Color,
    pub error_header: Color,
    pub error_content: Color,
}

impl From<&SessionDetailColorSettings> for SessionDetailTheme {
    fn from(colors: &SessionDetailColorSettings) -> Self {
        Self {
            border: configured_color(colors.border),
            popup_title: configured_color(colors.popup_title),
            metadata_key: configured_color(colors.metadata_key),
            metadata_value: configured_color(colors.metadata_value),
            conversation_header: configured_color(colors.conversation_header),
            empty_text: configured_color(colors.empty_text),
            status_success: configured_color(colors.status_success),
            status_error: configured_color(colors.status_error),
            footer_key: configured_color(colors.footer_key),
            footer_text: configured_color(colors.footer_text),
            footer_separator: configured_color(colors.footer_separator),
            user_header: configured_color(colors.user_header),
            user_content: configured_color(colors.user_content),
            assistant_header: configured_color(colors.assistant_header),
            assistant_content: configured_color(colors.assistant_content),
            skill_header: configured_color(colors.skill_header),
            skill_content: configured_color(colors.skill_content),
            tool_call_header: configured_color(colors.tool_call_header),
            tool_call_content: configured_color(colors.tool_call_content),
            tool_result_header: configured_color(colors.tool_result_header),
            tool_result_content: configured_color(colors.tool_result_content),
            system_header: configured_color(colors.system_header),
            system_content: configured_color(colors.system_content),
            error_header: configured_color(colors.error_header),
            error_content: configured_color(colors.error_content),
        }
    }
}

impl Default for SessionDetailTheme {
    fn default() -> Self {
        Self::from(&SessionDetailColorSettings::default())
    }
}

// ── Terminal lifecycle ─────────────────────────────────────────────────────────

pub(super) struct ManagedTerminal {
    pub terminal: DefaultTerminal,
    alternate_scroll: bool,
}

impl ManagedTerminal {
    pub(super) fn enter_with_native_selection() -> Result<Self> {
        Self::enter_internal(true)
    }

    fn enter_internal(alternate_scroll: bool) -> Result<Self> {
        let mut terminal = ratatui::try_init().context("failed to initialize terminal UI")?;
        if alternate_scroll
            && let Err(error) = configure_alternate_scroll(&mut std::io::stdout(), true)
        {
            let _ = ratatui::try_restore();
            return Err(error).context("failed to enable terminal alternate scrolling");
        }
        if let Err(error) = terminal.hide_cursor() {
            if alternate_scroll {
                let _ = configure_alternate_scroll(&mut std::io::stdout(), false);
            }
            let _ = ratatui::try_restore();
            return Err(error).context("failed to hide terminal cursor");
        }
        Ok(Self {
            terminal,
            alternate_scroll,
        })
    }
}

impl Drop for ManagedTerminal {
    fn drop(&mut self) {
        let _ = self.terminal.show_cursor();
        if self.alternate_scroll {
            let _ = configure_alternate_scroll(&mut std::io::stdout(), false);
        }
        let _ = ratatui::try_restore();
    }
}

fn configure_alternate_scroll(writer: &mut impl Write, enabled: bool) -> io::Result<()> {
    writer.write_all(if enabled {
        b"\x1b[?1007h"
    } else {
        b"\x1b[?1007l"
    })?;
    writer.flush()
}

// ── Input helpers ──────────────────────────────────────────────────────────────

pub(super) const fn is_key_press(key: &KeyEvent) -> bool {
    matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
}

// ── Key hint rendering ─────────────────────────────────────────────────────────

pub(super) fn key_hints(hints: &[(&str, &str)]) -> Line<'static> {
    themed_key_hints(hints, UI.signal, UI.text_soft, UI.grid)
}

pub(super) fn responsive_key_hints(
    width: u16,
    full: &[(&str, &str)],
    compact: &[(&str, &str)],
) -> Line<'static> {
    key_hints(if width >= 96 { full } else { compact })
}

pub(super) fn themed_key_hints(
    hints: &[(&str, &str)],
    key_color: Color,
    text_color: Color,
    separator_color: Color,
) -> Line<'static> {
    let mut spans = Vec::new();
    for (index, (key, action)) in hints.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled("   ", Style::default()));
        }
        spans.push(Span::styled(
            "[",
            Style::default().fg(separator_color).bg(UI.panel_alt),
        ));
        spans.push(Span::styled(
            (*key).to_owned(),
            Style::default()
                .fg(key_color)
                .bg(UI.panel_alt)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            "]",
            Style::default().fg(separator_color).bg(UI.panel_alt),
        ));
        spans.push(Span::styled(
            format!(" {action}"),
            Style::default().fg(text_color),
        ));
    }
    Line::from(spans)
}

// ── Layout helpers ─────────────────────────────────────────────────────────────

pub(super) fn centered_rect(area: Rect, preferred_width: u16, preferred_height: u16) -> Rect {
    let width = preferred_width.min(area.width.saturating_sub(2)).max(1);
    let height = preferred_height.min(area.height.saturating_sub(2)).max(1);
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}
// ── Header frame ──────────────────────────────────────────────────────────────

/// Render a quiet, static frame around a screen header.
pub(crate) fn render_header_frame(frame: &mut ratatui::Frame<'_>, area: Rect, title: &str) {
    if area.width < 4 || area.height < 4 {
        return;
    }

    let block = ratatui::widgets::Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(UI.border))
        .style(Style::default().bg(UI.panel).fg(UI.text))
        .title(Span::styled(
            title.to_owned(),
            Style::default().fg(UI.text_soft),
        ));
    frame.render_widget(block, area);
}

// ── Animation: Thinking Orbs (AI 思考/脉冲点阵球) ─────────────────────────────────

const ORB_PULSE_FRAMES: &[&str] = &[" ⠂⠄⠂ ", " ⠅⠤⠅ ", " ⣁⠶⣁ ", " ⣾⠽⣷ ", " ⣴⠾⣦ ", " ⠅⠤⠅ "];

pub(crate) fn thinking_orb_spans(tick: usize, label: &str) -> Vec<Span<'static>> {
    let pulse_idx = tick % ORB_PULSE_FRAMES.len();
    let orb_symbol = ORB_PULSE_FRAMES[pulse_idx];

    vec![
        Span::styled(orb_symbol.to_string(), Style::default().fg(UI.signal)),
        Span::styled(format!(" {label} "), Style::default().fg(UI.text_soft)),
    ]
}
