use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use crate::fs::atomic_create_private;
use crate::session::{
    DetailScope, ResponseMetrics, SessionDetail, SessionMessage, SessionMessageKind,
};
use crate::view::{
    TOOL_TOKEN_ACCOUNTING_NOTE, format_metric_error, format_model_usage_summary,
    format_response_header_metrics, format_response_summary, format_token_breakdown,
    format_tool_summary,
};

/// Export a session detail document into `directory` as Markdown.
///
/// `scope` selects which messages are written: `Conversation` omits tool
/// calls, tool results, skills, system, and error messages. The returned path
/// is absolute. Existing files are never replaced; an incrementing suffix is
/// added when a timestamped name already exists.
///
/// # Errors
///
/// Returns an error when the destination directory cannot be resolved or the
/// private atomic file creation fails.
pub fn export_session_detail(
    detail: &SessionDetail,
    directory: &Path,
    scope: DetailScope,
) -> Result<PathBuf> {
    export_session_detail_at(detail, directory, SystemTime::now(), scope)
}

pub fn render_session_detail_markdown(detail: &SessionDetail, scope: DetailScope) -> String {
    render_markdown(detail, SystemTime::now().into(), scope)
}

fn export_session_detail_at(
    detail: &SessionDetail,
    directory: &Path,
    exported_at: SystemTime,
    scope: DetailScope,
) -> Result<PathBuf> {
    let directory = directory
        .canonicalize()
        .with_context(|| format!("failed to resolve export directory {}", directory.display()))?;
    let exported_datetime: DateTime<Utc> = exported_at.into();
    let timestamp = exported_datetime.format("%Y%m%d-%H%M%S");
    let provider = sanitize_filename_component(detail.session.kind.slug(), "agent");
    let session_id = sanitize_filename_component(&detail.session.id, "session");
    let variant = match scope {
        DetailScope::All => "full",
        DetailScope::Conversation => "conv",
    };
    let stem = format!("mena-session-{provider}-{session_id}-{timestamp}-{variant}");
    let markdown = render_markdown(detail, exported_datetime, scope);

    for sequence in 1_usize.. {
        let suffix = if sequence == 1 {
            String::new()
        } else {
            format!("-{sequence}")
        };
        let path = directory.join(format!("{stem}{suffix}.md"));
        if atomic_create_private(&path, markdown.as_bytes())? {
            return Ok(path);
        }
    }
    unreachable!("an unbounded sequence always has another candidate file name")
}

fn render_markdown(
    detail: &SessionDetail,
    exported_at: DateTime<Utc>,
    scope: DetailScope,
) -> String {
    let mut markdown = String::from("# Mena Session Export\n\n");
    render_metadata(&mut markdown, detail, exported_at);
    render_model_usage(&mut markdown, detail);
    render_conversation(&mut markdown, detail, scope);
    markdown
}

fn render_metadata(markdown: &mut String, detail: &SessionDetail, exported_at: DateTime<Utc>) {
    let session = &detail.session;
    let target = session.target();
    let updated = format_unix_timestamp(session.updated_at);
    let tokens = session
        .tokens
        .map_or_else(|| "-".to_owned(), |value| value.to_string());
    let cost = session
        .cost_usd
        .map_or_else(|| "n/a".to_owned(), |value| format!("${value:.4}"));
    for (label, value) in [
        ("TARGET", target),
        ("Agent", session.kind.to_string()),
        (
            "Title",
            session.title.as_deref().unwrap_or("(untitled)").to_owned(),
        ),
        (
            "Project",
            session
                .project
                .as_deref()
                .map_or_else(|| "-".to_owned(), |path| path.display().to_string()),
        ),
        (
            "Started",
            session.started_at.as_deref().unwrap_or("-").to_owned(),
        ),
        ("Updated", updated),
        ("Tokens", tokens),
        ("Cost", cost),
        ("Log File", session.path.display().to_string()),
        ("Exported At", exported_at.to_rfc3339()),
    ] {
        let _ = writeln!(markdown, "- **{label}:** {}", one_line(&value));
    }
}

fn render_model_usage(markdown: &mut String, detail: &SessionDetail) {
    let model_usage = detail.model_usage();
    if !model_usage.is_empty() {
        markdown.push_str("\n## Model Usage\n\n");
        for summary in &model_usage {
            let _ = writeln!(markdown, "- {}", format_model_usage_summary(summary));
            if let Some(tokens) = format_token_breakdown(summary.tokens) {
                let _ = writeln!(markdown, "  - Token details: {tokens}");
            }
        }
    }
}

fn render_conversation(markdown: &mut String, detail: &SessionDetail, scope: DetailScope) {
    let visible: Vec<&SessionMessage> = detail.messages_in(scope).collect();
    let _ = write!(
        markdown,
        "\n## Conversation ({})\n\n{}",
        scope.label(),
        if detail.messages.is_empty() {
            "_No persisted chat messages were found for this session._\n".to_owned()
        } else if visible.is_empty() {
            "_No user or assistant messages in this scope._\n".to_owned()
        } else {
            String::new()
        }
    );
    let hidden = detail.hidden_message_count(scope);
    if hidden > 0 {
        let _ = writeln!(
            markdown,
            "_{hidden} tool/system message{} hidden by `{}` scope._\n",
            if hidden == 1 { "" } else { "s" },
            scope.label(),
        );
    }
    for message in visible {
        render_message(markdown, message);
    }
}

fn render_message(markdown: &mut String, message: &SessionMessage) {
    let timestamp = message.timestamp.as_deref().unwrap_or("-");
    let metrics = message_header_metrics(message);
    let _ = writeln!(
        markdown,
        "### [{timestamp}] {}{metrics}\n",
        message.kind.label()
    );
    if let Some(response) = message.metrics.response.as_ref() {
        render_response_metrics(markdown, response);
    }
    if matches!(
        message.kind,
        SessionMessageKind::Skill | SessionMessageKind::ToolCall
    ) {
        render_tool_metrics(markdown, message);
    }
    render_message_body(markdown, message);
    markdown.push('\n');
}

fn message_header_metrics(message: &SessionMessage) -> String {
    let mut metrics = String::new();
    if let Some(model) = message
        .model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        let _ = write!(metrics, " · {model}");
    }
    if let Some(response) = message.metrics.response.as_ref() {
        for metric in format_response_header_metrics(response) {
            let _ = write!(metrics, " · {metric}");
        }
    }
    if let Some(tool) = message.metrics.tool.as_ref()
        && let Some(summary) = format_tool_summary(tool)
    {
        let _ = write!(metrics, " · {summary}");
    }
    metrics
}

fn render_response_metrics(markdown: &mut String, response: &ResponseMetrics) {
    if let Some(tokens) = format_token_breakdown(response.tokens) {
        let _ = writeln!(markdown, "**Token details:** {tokens}\n");
    }
    if let Some(summary) = format_response_summary(response) {
        let _ = writeln!(markdown, "**Response:** {summary}\n");
    }
    if let Some(error) = response.error.as_ref() {
        let _ = writeln!(
            markdown,
            "**Error:** {}\n",
            one_line(&format_metric_error(error))
        );
    }
}

fn render_tool_metrics(markdown: &mut String, message: &SessionMessage) {
    if let Some(tool) = message.metrics.tool.as_ref() {
        if let Some(summary) = format_tool_summary(tool) {
            let _ = writeln!(markdown, "**Tool:** {summary}\n");
        }
        if let Some(error) = tool.error.as_ref() {
            let _ = writeln!(
                markdown,
                "**Error:** {}\n",
                one_line(&format_metric_error(error))
            );
        }
    }
    let _ = writeln!(markdown, "_{TOOL_TOKEN_ACCOUNTING_NOTE}_\n");
}

fn render_message_body(markdown: &mut String, message: &SessionMessage) {
    if matches!(
        message.kind,
        SessionMessageKind::Skill | SessionMessageKind::ToolCall | SessionMessageKind::ToolResult
    ) {
        let language = if serde_json::from_str::<serde_json::Value>(&message.content).is_ok() {
            "json"
        } else {
            ""
        };
        write_code_fence(markdown, &message.content, language);
    } else {
        markdown.push_str(&message.content);
        if !message.content.ends_with('\n') {
            markdown.push('\n');
        }
    }
}

fn write_code_fence(markdown: &mut String, content: &str, language: &str) {
    let fence = "`".repeat(longest_backtick_run(content).saturating_add(1).max(3));
    let _ = writeln!(markdown, "{fence}{language}");
    markdown.push_str(content);
    if !content.ends_with('\n') {
        markdown.push('\n');
    }
    let _ = writeln!(markdown, "{fence}");
}

fn longest_backtick_run(value: &str) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for character in value.chars() {
        if character == '`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

fn sanitize_filename_component(value: &str, fallback: &str) -> String {
    let mut sanitized = String::new();
    let mut last_was_separator = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
            sanitized.push(character);
            last_was_separator = false;
        } else if !last_was_separator {
            sanitized.push('_');
            last_was_separator = true;
        }
    }
    let sanitized = sanitized.trim_matches('_');
    if sanitized.is_empty() {
        fallback.to_owned()
    } else {
        sanitized.to_owned()
    }
}

fn one_line(value: &str) -> String {
    value.lines().collect::<Vec<_>>().join(" ")
}

fn format_unix_timestamp(timestamp: u64) -> String {
    i64::try_from(timestamp)
        .ok()
        .and_then(|timestamp| DateTime::from_timestamp(timestamp, 0))
        .map_or_else(|| timestamp.to_string(), |value| value.to_rfc3339())
}

#[cfg(test)]
mod tests;
