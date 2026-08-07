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
