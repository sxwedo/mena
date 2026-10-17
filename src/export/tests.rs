
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, UNIX_EPOCH};

use tempfile::tempdir;

use super::export_session_detail_at;
use crate::AgentKind;
use crate::session::{
    AgentSession, DetailScope, ResponseMetrics, SessionDetail, SessionMessage, SessionMessageKind,
    SessionMessageMetrics, TokenUsage, ToolMetrics,
};

#[test]
fn exports_complete_markdown_metadata_and_messages_to_unique_private_files() {
    let directory = tempdir().expect("export directory");
    let detail = fixture_detail();
    let exported_at = UNIX_EPOCH + Duration::from_secs(1_767_225_845);

    let first = export_session_detail_at(&detail, directory.path(), exported_at, DetailScope::All)
        .expect("first export");
    let second = export_session_detail_at(&detail, directory.path(), exported_at, DetailScope::All)
        .expect("collision export");

    assert!(first.is_absolute());
    assert_eq!(
        first.file_name().and_then(|value| value.to_str()),
        Some("mena-session-codex-id_with_unicode-20260101-000405-full.md")
    );
    assert_eq!(
        second.file_name().and_then(|value| value.to_str()),
        Some("mena-session-codex-id_with_unicode-20260101-000405-full-2.md")
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
        "ASSISTANT · gpt-5.5 · 12.3s · 67,890 tokens",
        "## Model Usage",
        "gpt-5.5 · 1 responses · duration 12.3s · avg TTFT 450ms · 67,890 tokens · $0.1250",
        "**Token details:** input 50,000 · output 10,000 · cache read 7,000 · cache write 500 (5m 400 · 1h 100) · reasoning 390 · tool 1,000",
        "**Response:** status completed · stop reason stop · TTFT 450ms · retries 1",
        "model-specific answer",
        "TOOL CALL · completed · 140ms · exit 0",
        "**Tool:** completed · 140ms · exit 0",
        "Token accounting: provider response totals only; no per-call token value is persisted.",
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

#[test]
fn conversation_scope_hides_tools_but_keeps_metadata_and_model_usage() {
    let directory = tempdir().expect("export directory");
    let detail = fixture_detail();
    let exported_at = UNIX_EPOCH + Duration::from_secs(1_767_225_845);

    let path = export_session_detail_at(
        &detail,
        directory.path(),
        exported_at,
        DetailScope::Conversation,
    )
    .expect("conversation export");

    assert_eq!(
        path.file_name().and_then(|value| value.to_str()),
        Some("mena-session-codex-id_with_unicode-20260101-000405-conv.md")
    );
    let markdown = fs::read_to_string(&path).expect("read conversation Markdown");

    // User/assistant dialogue is retained.
    assert!(markdown.contains("第一行\n第二行"));
    assert!(markdown.contains("ASSISTANT · gpt-5.5"));
    assert!(markdown.contains("model-specific answer"));
    // Metadata and model usage are always present, even in conversation scope.
    assert!(markdown.contains("TARGET"));
    assert!(markdown.contains("## Model Usage"));
    assert!(markdown.contains("gpt-5.5 · 1 responses"));
    // Tool content is hidden.
    assert!(!markdown.contains("TOOL CALL"));
    assert!(!markdown.contains("TOOL RESULT"));
    assert!(!markdown.contains("README.md"));
    assert!(!markdown.contains("最后一条，完整保留"));
    // The hidden count and scope are surfaced.
    assert!(markdown.contains("2 tool/system messages hidden by `conversation only` scope"));
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
                related_paths: BTreeSet::new(),
            },
            messages: vec![
                SessionMessage {
                    kind: SessionMessageKind::User,
                    timestamp: Some("2026-01-01T00:00:02Z".to_owned()),
                    model: None,
                    metrics: SessionMessageMetrics::default(),
                    content: "第一行\n第二行".to_owned(),
                },
                SessionMessage {
                    kind: SessionMessageKind::Assistant,
                    timestamp: Some("2026-01-01T00:00:02Z".to_owned()),
                    model: Some("gpt-5.5".to_owned()),
                    metrics: SessionMessageMetrics {
                        response: Some(ResponseMetrics {
                            duration_ms: Some(12_345),
                            time_to_first_token_ms: Some(450),
                            cost_usd: Some(0.125),
                            finish_reason: Some("stop".to_owned()),
                            retry_count: Some(1),
                            tokens: TokenUsage {
                                total: Some(67_890),
                                input: Some(50_000),
                                output: Some(10_000),
                                cache_read: Some(7_000),
                                cache_write: Some(500),
                                cache_write_5m: Some(400),
                                cache_write_1h: Some(100),
                                reasoning: Some(390),
                                tool: Some(1_000),
                            },
                            ..ResponseMetrics::default()
                        }),
                        ..SessionMessageMetrics::default()
                    },
                    content: "model-specific answer".to_owned(),
                },
                SessionMessage {
                    kind: SessionMessageKind::ToolCall,
                    timestamp: Some("2026-01-01T00:00:03Z".to_owned()),
                    model: None,
                    metrics: SessionMessageMetrics {
                        response: None,
                        tool: Some(ToolMetrics {
                            status: Some("completed".to_owned()),
                            duration_ms: Some(140),
                            exit_code: Some(0),
                            error: None,
                        }),
                    },
                    content: "{\n  \"name\": \"read\",\n  \"path\": \"README.md\",\n  \"literal\": \"```\"\n}".to_owned(),
                },
                SessionMessage {
                    kind: SessionMessageKind::ToolResult,
                    timestamp: None,
                    model: None,
                    metrics: SessionMessageMetrics::default(),
                    content: "最后一条，完整保留".to_owned(),
                },
            ],
        }
}
