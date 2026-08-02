use std::fmt::Write as _;

use unicode_width::UnicodeWidthStr;

use crate::process::LiveAgent;
use crate::session::AgentSession;

#[derive(Debug, Clone, PartialEq)]
pub struct AgentReport {
    pub agent: LiveAgent,
    pub session: Option<AgentSession>,
}

impl AgentReport {
    #[must_use]
    pub fn project(&self) -> Option<&std::path::Path> {
        self.session
            .as_ref()
            .and_then(|session| session.project.as_deref())
            .or(self.agent.process.cwd.as_deref())
    }
}

pub fn render_process_table(
    reports: &[AgentReport],
    resources: bool,
    selected: Option<usize>,
) -> String {
    if reports.is_empty() {
        return "No running developer agents found.\n".to_owned();
    }

    let mut headers = vec!["ID", "AGENT", "PROJECT", "STATUS", "DURATION"];
    if resources {
        headers.extend(["CPU", "MEMORY"]);
    }
    headers.extend(["TOKENS", "COST"]);

    let rows: Vec<Vec<String>> = reports
        .iter()
        .enumerate()
        .map(|(index, report)| {
            let agent = &report.agent;
            let mut row = vec![
                format!("{}:{}", agent.kind.slug(), agent.process.pid),
                agent.kind.to_string(),
                report
                    .project()
                    .map_or_else(|| "-".to_owned(), project_label),
                agent.process.status.clone(),
                format_duration(agent.process.run_time),
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
            if resources {
                row.push(format!("{:.1}%", agent.process.cpu_percent));
                row.push(format_bytes(agent.process.memory_bytes));
            }
            let tokens = report
                .session
                .as_ref()
                .and_then(|session| session.tokens)
                .map_or_else(|| "-".to_owned(), format_tokens);
            let cost = report.session.as_ref().map_or_else(
                || "-".to_owned(),
                |session| {
                    session
                        .cost_usd
                        .map_or_else(|| "n/a".to_owned(), |cost| format!("${cost:.4}"))
                },
            );
            row.extend([tokens, cost]);
            row
        })
        .collect();
    if selected.is_some() {
        let mut display_headers = Vec::with_capacity(headers.len() + 1);
        display_headers.push("");
        display_headers.extend(headers);
        render_table(&display_headers, &rows)
    } else {
        render_table(&headers, &rows)
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

fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format_compact(tokens, 1_000_000, "M")
    } else if tokens >= 1_000 {
        format_compact(tokens, 1_000, "K")
    } else {
        tokens.to_string()
    }
}

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

fn format_scaled(bytes: u64, unit: u64, suffix: &str) -> String {
    let whole = bytes / unit;
    let decimal = bytes % unit * 10 / unit;
    format!("{whole}.{decimal} {suffix}")
}

#[cfg(test)]
mod tests {
    use super::{format_count, format_duration_millis};

    #[test]
    fn formats_exact_counts_and_subsecond_response_durations() {
        assert_eq!(format_count(123_456_789), "123,456,789");
        assert_eq!(format_duration_millis(450), "450ms");
        assert_eq!(format_duration_millis(12_345), "12.3s");
        assert_eq!(format_duration_millis(125_450), "2m 05.5s");
        assert_eq!(format_duration_millis(3_725_450), "1h 02m 05.5s");
    }
}
