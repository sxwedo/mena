//! Shared terminal primitives and styling helpers used across TUI modules.

use std::io::{self, Write};

use anyhow::{Context, Result};
use crossterm::event::{KeyEvent, KeyEventKind};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::{DefaultTerminal, layout::Rect};

use crate::settings::{ConfigColor, SessionDetailColorSettings};

// ── Palette constants ──────────────────────────────────────────────────────────

pub(super) const ACCENT: Color = Color::Cyan;
pub(super) const MUTED: Color = Color::DarkGray;
#[allow(dead_code)]
pub(super) const METADATA_KEY: Color = Color::LightMagenta;

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
    #[allow(dead_code)]
    pub(super) fn enter() -> Result<Self> {
        Self::enter_internal(false)
    }

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
    themed_key_hints(hints, ACCENT, Color::Reset, MUTED)
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
            spans.push(Span::styled("  •  ", Style::default().fg(separator_color)));
        }
        spans.push(Span::styled(
            (*key).to_owned(),
            Style::default().fg(key_color).add_modifier(Modifier::BOLD),
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
// ── Animation: Border Beam (流光边框) ─────────────────────────────────────────

/// Render a glowing border beam animated around `area` at tick `tick`.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::suboptimal_flops
)]
pub(crate) fn render_border_beam(
    frame: &mut ratatui::Frame<'_>,
    area: Rect,
    tick: usize,
    title: &str,
    base_color: Color,
    beam_color: Color,
) {
    if area.width < 3 || area.height < 3 {
        return;
    }

    let w = usize::from(area.width);
    let h = usize::from(area.height);
    let perimeter = (w * 2) + (h * 2) - 4;
    if perimeter == 0 {
        return;
    }

    let beam_length = (perimeter / 4).max(6);
    let speed = 2;
    let head = (tick * speed) % perimeter;

    let calc_fade = |pos: usize| -> Option<f32> {
        let dist = if pos <= head {
            head - pos
        } else {
            perimeter + head - pos
        };
        if dist <= beam_length {
            Some(1.0 - (dist as f32 / beam_length as f32))
        } else {
            None
        }
    };

    let block = ratatui::widgets::Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .border_style(Style::default().fg(base_color))
        .title(title);

    frame.render_widget(block, area);

    let mut top_spans = Vec::with_capacity(w);
    for x in 0..w {
        let pos = x;
        let ch = if x == 0 {
            "╭"
        } else if x == w - 1 {
            "╮"
        } else {
            "─"
        };
        if let Some(intensity) = calc_fade(pos) {
            let (br, bg, bb) = match beam_color {
                Color::Cyan => (0.0, 255.0, 255.0),
                Color::Yellow => (255.0, 220.0, 0.0),
                Color::Green => (0.0, 255.0, 120.0),
                _ => (255.0, 255.0, 255.0),
            };
            let r = (br * intensity + 40.0 * (1.0 - intensity)) as u8;
            let g = (bg * intensity + 45.0 * (1.0 - intensity)) as u8;
            let b = (bb * intensity + 55.0 * (1.0 - intensity)) as u8;
            top_spans.push(Span::styled(
                ch,
                Style::default()
                    .fg(Color::Rgb(r, g, b))
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            top_spans.push(Span::styled(ch, Style::default().fg(base_color)));
        }
    }

    let top_rect = Rect::new(area.x, area.y, area.width, 1);
    frame.render_widget(
        ratatui::widgets::Paragraph::new(Line::from(top_spans)),
        top_rect,
    );
}

// ── Animation: Thinking Orbs (AI 思考/脉冲点阵球) ─────────────────────────────────

const ORB_PULSE_FRAMES: &[&str] = &[" ⠂⠄⠂ ", " ⠅⠤⠅ ", " ⣁⠶⣁ ", " ⣾⠽⣷ ", " ⣴⠾⣦ ", " ⠅⠤⠅ "];

pub(crate) fn thinking_orb_spans(tick: usize, label: &str) -> Vec<Span<'static>> {
    let pulse_idx = tick % ORB_PULSE_FRAMES.len();
    let orb_symbol = ORB_PULSE_FRAMES[pulse_idx];

    let color_cycle = match tick % 3 {
        0 => Color::Cyan,
        1 => Color::Yellow,
        _ => Color::Green,
    };

    vec![
        Span::styled(
            orb_symbol.to_string(),
            Style::default()
                .fg(color_cycle)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {label} "),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
    ]
}
