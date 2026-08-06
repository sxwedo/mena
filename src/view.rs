use std::fmt::Write as _;

use unicode_width::UnicodeWidthStr;

use crate::process::LiveAgent;
use crate::session::{
    AgentSession, AssociationSummary, MetricError, ModelUsageSummary, ResponseMetrics, TokenUsage,
    ToolMetrics,
};
use crate::skill::{AgentSkill, SkillDetail};

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq)]
pub struct AgentReport {
    pub agent: LiveAgent,
    pub session: Option<AgentSession>,
    pub association: AssociationSummary,
}

impl AgentReport {
    #[allow(dead_code)]
    #[must_use]
    pub fn project(&self) -> Option<&std::path::Path> {
        self.session
            .as_ref()
            .and_then(|session| session.project.as_deref())
            .or(self.agent.process.cwd.as_deref())
    }
}

pub fn render_session_table(sessions: &[AgentSession], selected: Option<usize>) -> String {
    if sessions.is_empty() {
        return "No saved developer-agent sessions found.\n".to_owned();
    }

    let headers = ["TARGET", "AGENT", "PROJECT", "TITLE / SUMMARY", "UPDATED"];
    let rows: Vec<Vec<String>> = sessions
        .iter()
        .enumerate()
        .map(|(index, session)| {
            let mut row = vec![
                session.target(),
                session.kind.to_string(),
                session
                    .project
                    .as_deref()
                    .map_or_else(|| "-".to_owned(), project_label),
                session
                    .title
                    .clone()
                    .unwrap_or_else(|| "(untitled)".to_owned()),
                format_age(session.updated_at),
            ];
            if let Some(selected) = selected {
                row.insert(
                    0,
                    if selected == index {
                        ">".to_owned()
                    } else {
                        " ".to_owned()
                    },
                );
            }
            row
        })
        .collect();
    if selected.is_some() {
        render_table(
            &[
                "",
                "TARGET",
                "AGENT",
                "PROJECT",
                "TITLE / SUMMARY",
                "UPDATED",
            ],
            &rows,
        )
    } else {
        render_table(&headers, &rows)
    }
}

pub fn render_skill_table(skills: &[AgentSkill]) -> String {
    let bold_cyan = anstyle::Style::new()
        .fg_color(Some(anstyle::Color::Ansi(anstyle::AnsiColor::Cyan)))
        .bold();
    let green =
        anstyle::Style::new().fg_color(Some(anstyle::Color::Ansi(anstyle::AnsiColor::Green)));
    let cyan = anstyle::Style::new().fg_color(Some(anstyle::Color::Ansi(anstyle::AnsiColor::Cyan)));
    let magenta =
        anstyle::Style::new().fg_color(Some(anstyle::Color::Ansi(anstyle::AnsiColor::Magenta)));
    let yellow =
        anstyle::Style::new().fg_color(Some(anstyle::Color::Ansi(anstyle::AnsiColor::Yellow)));
    let reset = anstyle::Style::new();

    let headers = [
        "NAME",
        "PROVIDER",
        "TYPE",
        "LOCATION",
        "SCOPE",
        "TRIGGERS",
        "DESCRIPTION",
    ];
    let rows: Vec<Vec<String>> = skills
        .iter()
        .map(|skill| {
            let triggers = if skill.triggers.is_empty() {
                "-".to_string()
            } else {
                skill.triggers.join(", ")
            };
            let description = skill
                .description
                .as_deref()
                .unwrap_or("-")
                .split('\n')
                .next()
                .unwrap_or("-")
                .to_string();

            let scope_styled = if skill.scope == "global" {
                format!("{cyan}[{}]{reset}", skill.scope)
            } else {
                format!("{green}[{}]{reset}", skill.scope)
            };

            let provider_styled = match skill.provider.as_str() {
                "claude" => format!("{magenta}[{}]{reset}", skill.provider),
                "codex" => format!("{yellow}[{}]{reset}", skill.provider),
                _ => format!("[{}]", skill.provider),
            };

            let type_styled = if skill.is_symlink {
                format!("{yellow}[symlink]{reset}")
            } else {
                "[file]".to_string()
            };

            vec![
                format!("{bold_cyan}{}{reset}", skill.name),
                provider_styled,
                type_styled,
                skill.location.clone(),
                scope_styled,
                triggers,
                description,
            ]
        })
        .collect();
    render_table(&headers, &rows)
}

pub fn render_skill_detail(detail: &SkillDetail) -> String {
    let bold_magenta = anstyle::Style::new()
        .fg_color(Some(anstyle::Color::Ansi(anstyle::AnsiColor::Magenta)))
        .bold();
    let bold_cyan = anstyle::Style::new()
        .fg_color(Some(anstyle::Color::Ansi(anstyle::AnsiColor::Cyan)))
        .bold();
    let green =
        anstyle::Style::new().fg_color(Some(anstyle::Color::Ansi(anstyle::AnsiColor::Green)));
    let red = anstyle::Style::new().fg_color(Some(anstyle::Color::Ansi(anstyle::AnsiColor::Red)));
    let dim = anstyle::Style::new().dimmed();
    let reset = anstyle::Style::new();

    let mut out = String::new();
    let s = &detail.skill;

    let _ = writeln!(
        out,
        "╭─────────────────────────────────────────────────────────────"
    );
    let _ = writeln!(out, "│ {bold_cyan}Skill Inspector:{reset} {}", s.name);
    let _ = writeln!(
        out,
        "├─────────────────────────────────────────────────────────────"
    );
    let _ = writeln!(out, "│ {bold_magenta}Provider:{reset}    {}", s.provider);
    let _ = writeln!(out, "│ {bold_magenta}Scope:{reset}       {}", s.scope);
    let _ = writeln!(
        out,
        "│ {bold_magenta}Path:{reset}        {dim}{}{reset}",
        s.path.display()
    );
    let _ = writeln!(
        out,
        "│ {bold_magenta}Valid:{reset}       {}",
        if s.valid {
            format!("{green}✓ true{reset}")
        } else {
            format!("{red}✗ false{reset}")
        }
    );
    if !s.triggers.is_empty() {
        let _ = writeln!(
            out,
            "│ {bold_magenta}Triggers:{reset}    {}",
            s.triggers.join(", ")
        );
    }
    if let Some(desc) = &s.description {
        let _ = writeln!(out, "│ {bold_magenta}Description:{reset} {desc}");
    }
    let _ = writeln!(
        out,
        "╰─────────────────────────────────────────────────────────────\n"
    );
    let _ = writeln!(
        out,
        "{dim}--- Content Preview ---{reset}\n{}",
        detail.content
    );
    out
}

fn render_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers
        .iter()
        .map(|header| UnicodeWidthStr::width(*header))
        .collect();
    for row in rows {
        for (index, value) in row.iter().enumerate() {
            widths[index] = widths[index].max(UnicodeWidthStr::width(value.as_str()));
        }
    }

    let mut output = String::new();
    for (index, header) in headers.iter().enumerate() {
        if index > 0 {
            output.push_str("  ");
        }
        write_cell(&mut output, header, widths[index]);
    }
    output.push('\n');
    for row in rows {
        for (index, value) in row.iter().enumerate() {
            if index > 0 {
                output.push_str("  ");
            }
            write_cell(&mut output, value, widths[index]);
        }
        output.push('\n');
    }
    output
}

fn write_cell(output: &mut String, value: &str, width: usize) {
    output.push_str(value);
    let padding = width.saturating_sub(UnicodeWidthStr::width(value));
    let _ = write!(output, "{:padding$}", "");
}

fn project_label(path: &std::path::Path) -> String {
    path.file_name()
        .filter(|name| !name.is_empty())
        .map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
}

#[allow(dead_code)]
fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format_compact(tokens, 1_000_000, "M")
    } else if tokens >= 1_000 {
        format_compact(tokens, 1_000, "K")
    } else {
        tokens.to_string()
    }
}

#[allow(dead_code)]
fn format_compact(value: u64, unit: u64, suffix: &str) -> String {
    let mut whole = value / unit;
    let mut decimal = (value % unit * 10 + unit / 2) / unit;
    if decimal == 10 {
        whole += 1;
        decimal = 0;
    }
    format!("{whole}.{decimal}{suffix}")
}

fn format_age(updated_at: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(updated_at, |duration| duration.as_secs());
    format!("{} ago", format_duration(now.saturating_sub(updated_at)))
}

pub fn format_duration(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = seconds % 86_400 / 3_600;
    let minutes = seconds % 3_600 / 60;
    let seconds = seconds % 60;
    if days > 0 {
        format!("{days}d {hours:02}h")
    } else if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

pub fn format_duration_millis(milliseconds: u64) -> String {
    if milliseconds < 1_000 {
        return format!("{milliseconds}ms");
    }
    let total_tenths = milliseconds.saturating_add(50) / 100;
    let total_seconds = total_tenths / 10;
    let tenths = total_tenths % 10;
    let hours = total_seconds / 3_600;
    let minutes = total_seconds % 3_600 / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m {seconds:02}.{tenths}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}.{tenths}s")
    } else {
        format!("{seconds}.{tenths}s")
    }
}

pub fn format_count(value: u64) -> String {
    let digits = value.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            formatted.push(',');
        }
        formatted.push(digit);
    }
    formatted
}

pub fn format_token_breakdown(usage: TokenUsage) -> Option<String> {
    let mut parts = Vec::new();
    for (label, value) in [
        ("input", usage.input),
        ("output", usage.output),
        ("cache read", usage.cache_read),
    ] {
        if let Some(value) = value {
            parts.push(format!("{label} {}", format_count(value)));
        }
    }
    if let Some(cache_write) = usage.cache_write {
        let mut detail = Vec::new();
        for (label, value) in [("5m", usage.cache_write_5m), ("1h", usage.cache_write_1h)] {
            if let Some(value) = value {
                detail.push(format!("{label} {}", format_count(value)));
            }
        }
        let detail = if detail.is_empty() {
            String::new()
        } else {
            format!(" ({})", detail.join(" · "))
        };
        parts.push(format!("cache write {}{detail}", format_count(cache_write)));
    }
    for (label, value) in [("reasoning", usage.reasoning), ("tool", usage.tool)] {
        if let Some(value) = value {
            parts.push(format!("{label} {}", format_count(value)));
        }
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

pub fn format_response_header_metrics(response: &ResponseMetrics) -> Vec<String> {
    let mut parts = Vec::new();
    if let Some(duration_ms) = response.duration_ms {
        parts.push(format_duration_millis(duration_ms));
    }
    if let Some(tokens) = response.tokens.total {
        parts.push(format!("{} tokens", format_count(tokens)));
    }
    if let Some(cost) = response.cost_usd {
        parts.push(format!("${cost:.4}"));
    }
    parts
}

pub fn format_response_summary(response: &ResponseMetrics) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(status) = response_status(response) {
        parts.push(format!("status {status}"));
    }
    if let Some(reason) = response.finish_reason.as_deref() {
        parts.push(format!("stop reason {reason}"));
    }
    if let Some(ttft) = response.time_to_first_token_ms {
        parts.push(format!("TTFT {}", format_duration_millis(ttft)));
    }
    if let Some(retries) = response.retry_count {
        parts.push(format!("retries {}", format_count(retries)));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

pub fn format_tool_summary(tool: &ToolMetrics) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(status) = tool.status.as_deref() {
        parts.push(status.to_owned());
    }
    if let Some(duration_ms) = tool.duration_ms {
        parts.push(format_duration_millis(duration_ms));
    }
    if let Some(exit_code) = tool.exit_code {
        parts.push(format!("exit {exit_code}"));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

pub fn format_metric_error(error: &MetricError) -> String {
    error.code.as_deref().map_or_else(
        || error.message.clone(),
        |code| format!("{code} · {}", error.message),
    )
}

pub fn format_model_usage_summary(summary: &ModelUsageSummary) -> String {
    let mut parts = vec![
        summary.model.clone(),
        format!("{} responses", format_count(summary.responses)),
    ];
    if let Some(duration_ms) = summary.duration_ms {
        parts.push(format!("duration {}", format_duration_millis(duration_ms)));
    }
    if let Some(ttft) = summary.average_time_to_first_token_ms {
        parts.push(format!("avg TTFT {}", format_duration_millis(ttft)));
    }
    if let Some(tokens) = summary.tokens.total {
        parts.push(format!("{} tokens", format_count(tokens)));
    }
    if let Some(cost) = summary.cost_usd {
        parts.push(format!("${cost:.4}"));
    }
    if let Some(retries) = summary.retry_count {
        parts.push(format!("{} retries", format_count(retries)));
    }
    if summary.errors > 0 {
        parts.push(format!("{} errors", format_count(summary.errors)));
    }
    parts.join(" · ")
}

pub const TOOL_TOKEN_ACCOUNTING_NOTE: &str =
    "Token accounting: provider response totals only; no per-call token value is persisted.";

fn response_status(response: &ResponseMetrics) -> Option<&'static str> {
    if response.error.is_some() {
        return Some("error");
    }
    let reason = response.finish_reason.as_deref()?.to_ascii_lowercase();
    match reason.as_str() {
        "task_complete" | "complete" | "completed" | "stop" | "end_turn" | "success" => {
            Some("completed")
        }
        "tool_use" | "tool_calls" | "tool-calls" | "function_call" => Some("tool use"),
        "max_tokens" | "length" | "length_limit" => Some("length limit"),
        "turn_aborted" | "aborted" | "interrupt" | "interrupted" | "user_interrupt" => {
            Some("interrupted")
        }
        "error" | "failed" | "failure" => Some("error"),
        "cancelled" | "canceled" => Some("cancelled"),
        _ => None,
    }
}

#[allow(dead_code)]
pub fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1_024;
    const MIB: u64 = KIB * 1_024;
    const GIB: u64 = MIB * 1_024;
    if bytes >= GIB {
        format_scaled(bytes, GIB, "GiB")
    } else if bytes >= MIB {
        format_scaled(bytes, MIB, "MiB")
    } else if bytes >= KIB {
        format_scaled(bytes, KIB, "KiB")
    } else {
        format!("{bytes} B")
    }
}

#[allow(dead_code)]
fn format_scaled(bytes: u64, unit: u64, suffix: &str) -> String {
    let whole = bytes / unit;
    let decimal = bytes % unit * 10 / unit;
    format!("{whole}.{decimal} {suffix}")
}

#[cfg(test)]
mod tests {
    use super::{
        format_count, format_duration_millis, format_metric_error, format_response_header_metrics,
        format_response_summary, format_token_breakdown, format_tool_summary,
    };
    use crate::session::{MetricError, ResponseMetrics, TokenUsage, ToolMetrics};

    #[test]
    fn formats_exact_counts_and_subsecond_response_durations() {
        assert_eq!(format_count(123_456_789), "123,456,789");
        assert_eq!(format_duration_millis(450), "450ms");
        assert_eq!(format_duration_millis(12_345), "12.3s");
        assert_eq!(format_duration_millis(125_450), "2m 05.5s");
        assert_eq!(format_duration_millis(3_725_450), "1h 02m 05.5s");
    }

    #[test]
    fn formats_only_persisted_token_breakdown_fields() {
        assert_eq!(
            format_token_breakdown(TokenUsage {
                total: Some(100),
                input: Some(70),
                output: Some(30),
                cache_read: Some(0),
                cache_write: None,
                cache_write_5m: None,
                cache_write_1h: None,
                reasoning: None,
                tool: None,
            })
            .as_deref(),
            Some("input 70 · output 30 · cache read 0")
        );
        assert_eq!(
            format_token_breakdown(TokenUsage {
                total: Some(100),
                ..TokenUsage::default()
            }),
            None
        );
    }

    #[test]
    fn formats_response_and_tool_metrics_without_inventing_missing_values() {
        let error = MetricError {
            code: Some("rate_limit".to_owned()),
            message: "retry budget exhausted".to_owned(),
        };
        let response = ResponseMetrics {
            duration_ms: Some(12_345),
            time_to_first_token_ms: Some(450),
            cost_usd: Some(0.125),
            finish_reason: Some("error".to_owned()),
            retry_count: Some(2),
            error: Some(error.clone()),
            tokens: TokenUsage {
                total: Some(67_890),
                ..TokenUsage::default()
            },
        };

        assert_eq!(
            format_response_header_metrics(&response),
            ["12.3s", "67,890 tokens", "$0.1250"]
        );
        assert_eq!(
            format_response_summary(&response).as_deref(),
            Some("status error · stop reason error · TTFT 450ms · retries 2")
        );
        assert_eq!(
            format_metric_error(&error),
            "rate_limit · retry budget exhausted"
        );
        assert_eq!(
            format_tool_summary(&ToolMetrics {
                status: Some("completed".to_owned()),
                duration_ms: Some(140),
                exit_code: Some(0),
                error: None,
            })
            .as_deref(),
            Some("completed · 140ms · exit 0")
        );
        assert_eq!(format_tool_summary(&ToolMetrics::default()), None);
    }
}
