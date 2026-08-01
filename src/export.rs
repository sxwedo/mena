use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use crate::fs::atomic_create_private;
use crate::session::{SessionDetail, SessionMessageKind};

/// Export a complete session detail document into `directory` as Markdown.
///
/// The returned path is absolute. Existing files are never replaced; an
/// incrementing suffix is added when a timestamped name already exists.
///
/// # Errors
///
/// Returns an error when the destination directory cannot be resolved or the
/// private atomic file creation fails.
pub fn export_session_detail(detail: &SessionDetail, directory: &Path) -> Result<PathBuf> {
    export_session_detail_at(detail, directory, SystemTime::now())
}

fn export_session_detail_at(
    detail: &SessionDetail,
    directory: &Path,
    exported_at: SystemTime,
) -> Result<PathBuf> {
    let directory = directory
        .canonicalize()
        .with_context(|| format!("failed to resolve export directory {}", directory.display()))?;
    let exported_datetime: DateTime<Utc> = exported_at.into();
    let timestamp = exported_datetime.format("%Y%m%d-%H%M%S");
    let provider = sanitize_filename_component(detail.session.kind.slug(), "agent");
    let session_id = sanitize_filename_component(&detail.session.id, "session");
    let stem = format!("mena-session-{provider}-{session_id}-{timestamp}");
    let markdown = render_markdown(detail, exported_datetime);

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

fn render_markdown(detail: &SessionDetail, exported_at: DateTime<Utc>) -> String {
    let session = &detail.session;
    let mut markdown = String::from("# Mena Session Export\n\n");
    let target = format!("{}:{}", session.kind.slug(), session.id);
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

    let _ = write!(
        markdown,
        "\n## Conversation\n\n{}",
        if detail.messages.is_empty() {
            "_No persisted chat messages were found for this session._\n".to_owned()
        } else {
            String::new()
        }
    );
    for message in &detail.messages {
        let timestamp = message.timestamp.as_deref().unwrap_or("-");
        let model = (message.kind == SessionMessageKind::Assistant)
            .then(|| message.model.as_deref().map(str::trim))
            .flatten()
            .filter(|model| !model.is_empty())
            .map_or_else(String::new, |model| format!(" · {model}"));
        let _ = writeln!(
            markdown,
            "### [{timestamp}] {}{model}\n",
            message.kind.label()
        );
        if matches!(
            message.kind,
            SessionMessageKind::Skill
                | SessionMessageKind::ToolCall
                | SessionMessageKind::ToolResult
        ) {
            let language = if serde_json::from_str::<serde_json::Value>(&message.content).is_ok() {
                "json"
            } else {
                ""
            };
            write_code_fence(&mut markdown, &message.content, language);
        } else {
            markdown.push_str(&message.content);
            if !message.content.ends_with('\n') {
                markdown.push('\n');
            }
        }
        markdown.push('\n');
    }
    markdown
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
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, UNIX_EPOCH};

    use tempfile::tempdir;

    use super::export_session_detail_at;
    use crate::AgentKind;
    use crate::session::{AgentSession, SessionDetail, SessionMessage, SessionMessageKind};

    #[test]
    fn exports_complete_markdown_metadata_and_messages_to_unique_private_files() {
        let directory = tempdir().expect("export directory");
        let detail = fixture_detail();
        let exported_at = UNIX_EPOCH + Duration::from_secs(1_767_225_845);

        let first =
            export_session_detail_at(&detail, directory.path(), exported_at).expect("first export");
        let second = export_session_detail_at(&detail, directory.path(), exported_at)
            .expect("collision export");

        assert!(first.is_absolute());
        assert_eq!(
            first.file_name().and_then(|value| value.to_str()),
            Some("mena-session-codex-id_with_unicode-20260101-000405.md")
        );
        assert_eq!(
            second.file_name().and_then(|value| value.to_str()),
            Some("mena-session-codex-id_with_unicode-20260101-000405-2.md")
        );
        let markdown = fs::read_to_string(&first).expect("read Markdown");
        for expected in [
            "TARGET",
            "codex:id/with unicode",
            "Agent",
            "Codex",
            "Title",
            "标题 | complete",
            "Project",
            "/work/项目",
            "Started",
            "2026-01-01T00:00:00Z",
            "Updated",
            "2026-01-01T00:00:01+00:00",
            "Tokens",
            "123456",
            "Cost",
            "$1.2500",
            "Log File",
            "/logs/session.jsonl",
            "Exported At",
            "2026-01-01T00:04:05+00:00",
            "USER",
            "第一行\n第二行",
            "ASSISTANT · gpt-5.5",
            "model-specific answer",
            "TOOL CALL",
            "README.md",
            "TOOL RESULT",
            "最后一条，完整保留",
        ] {
            assert!(
                markdown.contains(expected),
                "missing {expected:?}\n{markdown}"
            );
        }
        assert!(markdown.contains("````json"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            assert_eq!(
                fs::metadata(&first).expect("metadata").permissions().mode() & 0o777,
                0o600
            );
        }
    }

    fn fixture_detail() -> SessionDetail {
        SessionDetail {
            session: AgentSession {
                kind: AgentKind::Codex,
                id: "id/with unicode".to_owned(),
                title: Some("标题 | complete".to_owned()),
                project: Some(PathBuf::from("/work/项目")),
                path: PathBuf::from("/logs/session.jsonl"),
                started_at: Some("2026-01-01T00:00:00Z".to_owned()),
                updated_at: 1_767_225_601,
                tokens: Some(123_456),
                cost_usd: Some(1.25),
            },
            messages: vec![
                SessionMessage {
                    kind: SessionMessageKind::User,
                    timestamp: Some("2026-01-01T00:00:02Z".to_owned()),
                    model: None,
                    content: "第一行\n第二行".to_owned(),
                },
                SessionMessage {
                    kind: SessionMessageKind::Assistant,
                    timestamp: Some("2026-01-01T00:00:02Z".to_owned()),
                    model: Some("gpt-5.5".to_owned()),
                    content: "model-specific answer".to_owned(),
                },
                SessionMessage {
                    kind: SessionMessageKind::ToolCall,
                    timestamp: Some("2026-01-01T00:00:03Z".to_owned()),
                    model: None,
                    content: "{\n  \"name\": \"read\",\n  \"path\": \"README.md\",\n  \"literal\": \"```\"\n}".to_owned(),
                },
                SessionMessage {
                    kind: SessionMessageKind::ToolResult,
                    timestamp: None,
                    model: None,
                    content: "最后一条，完整保留".to_owned(),
                },
            ],
        }
    }
}
