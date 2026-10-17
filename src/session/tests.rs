use std::collections::BTreeSet;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::symlink;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use tempfile::tempdir;

use super::{
    AgentSession, AssociationDecision, AssociationEvidence, ResponseMetrics, SessionCatalog,
    SessionDetail, SessionMessage, SessionMessageKind, SessionMessageMetrics, TokenUsage,
};
use crate::process::{AgentKind, LiveAgent, ProcessSnapshot};

#[test]
fn aggregates_only_exact_response_metrics_by_model() {
    let response = |input: u64,
                    output: u64,
                    tool: u64,
                    duration: u64,
                    ttft: u64,
                    cost: f64,
                    retries: u64,
                    error: bool| {
        ResponseMetrics {
            duration_ms: Some(duration),
            time_to_first_token_ms: Some(ttft),
            cost_usd: Some(cost),
            retry_count: Some(retries),
            error: error.then(|| super::MetricError {
                code: Some("provider_error".to_owned()),
                message: "failed".to_owned(),
            }),
            tokens: TokenUsage {
                total: Some(input + output + tool),
                input: Some(input),
                output: Some(output),
                tool: Some(tool),
                ..TokenUsage::default()
            },
            ..ResponseMetrics::default()
        }
    };
    let detail = SessionDetail {
        session: AgentSession {
            kind: AgentKind::GeminiCli,
            id: "model-summary".to_owned(),
            title: None,
            project: None,
            path: PathBuf::from("session.json"),
            started_at: None,
            updated_at: 0,
            tokens: None,
            cost_usd: None,
            related_paths: BTreeSet::new(),
        },
        messages: vec![
            SessionMessage {
                kind: SessionMessageKind::Assistant,
                timestamp: None,
                model: Some("gemini-pro".to_owned()),
                metrics: SessionMessageMetrics {
                    response: Some(response(10, 2, 3, 1_000, 100, 0.1, 0, false)),
                    tool: None,
                },
                content: "first".to_owned(),
            },
            SessionMessage {
                kind: SessionMessageKind::Assistant,
                timestamp: None,
                model: Some("gemini-pro".to_owned()),
                metrics: SessionMessageMetrics {
                    response: Some(response(20, 4, 5, 2_000, 300, 0.2, 1, true)),
                    tool: None,
                },
                content: "second".to_owned(),
            },
        ],
    };

    let summaries = detail.model_usage();

    assert_eq!(summaries.len(), 1);
    let summary = &summaries[0];
    assert_eq!(summary.model, "gemini-pro");
    assert_eq!(summary.responses, 2);
    assert_eq!(summary.duration_ms, Some(3_000));
    assert_eq!(summary.average_time_to_first_token_ms, Some(200));
    assert_eq!(summary.retry_count, Some(1));
    assert_eq!(summary.errors, 1);
    assert_eq!(summary.tokens.total, Some(44));
    assert_eq!(summary.tokens.tool, Some(8));
    assert!((summary.cost_usd.expect("cost") - 0.3).abs() < f64::EPSILON);
}

#[test]
fn duplicate_native_records_resolve_to_one_logical_session() {
    let temp = tempdir().expect("temp home");
    let session_id = "019fb342-b647-78f3-9391-365724790c7e";
    let mut rollout_paths = Vec::new();
    for date in ["2026/07/30", "2026/07/31"] {
        let path = temp
            .path()
            .join(format!(".codex/sessions/{date}/duplicate.jsonl"));
        write(
            &path,
            &format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{session_id}\",\"cwd\":\"/work/mena\"}}}}\n"
            ),
        );
        rollout_paths.push(path);
    }
    let catalog = SessionCatalog::scan(temp.path()).expect("scan duplicate records");

    // The same logical session spans two rollout files but is listed
    // once: one file is the primary `path`, the other joins
    // `related_paths`. (Both are written within the same second, so
    // either file may sort first.)
    assert_eq!(catalog.sessions().len(), 1);
    let session = &catalog.sessions()[0];
    assert_eq!(session.kind, AgentKind::Codex);
    assert_eq!(session.id, session_id);
    assert_eq!(session.related_paths.len(), 1);
    for path in &rollout_paths {
        assert!(
            session.path == *path || session.related_paths.contains(path),
            "{} must be cataloged exactly once",
            path.display()
        );
    }

    let resolved = catalog
        .resolve(Some("codex"), session_id)
        .expect("a provider-qualified full ID identifies one logical session");

    assert_eq!(resolved.id, session_id);
    assert_eq!(resolved.kind, AgentKind::Codex);

    // Deleting the one row removes every native file of the session.
    let summary = catalog
        .delete_session(resolved)
        .expect("delete the logical session");
    assert_eq!(summary.files, 2);
    for path in &rollout_paths {
        assert!(!path.exists(), "{} should be removed", path.display());
    }
}

#[test]
fn claude_subagent_transcripts_do_not_duplicate_the_parent_session() {
    let temp = tempdir().expect("temp home");
    let home = temp.path();
    let project = home.join(".claude/projects/-Users-test-project");
    fs::create_dir_all(project.join("49e35944/subagents")).expect("create subagents dir");

    let parent_id = "49e35944-03f5-4c90-98b9-12688f3bb3b1";
    write(
        &project.join(format!("{parent_id}.jsonl")),
        &format!(
            "{{\"sessionId\":\"{parent_id}\",\"cwd\":\"/test/project\",\"message\":{{\"role\":\"user\",\"content\":\"Plan the refactor\"}}}}\n"
        ),
    );
    // Subagent files carry the parent session ID; they must not appear
    // as their own rows.
    write(
        &project.join("49e35944/subagents/agent-af4800c76f85cfb61.jsonl"),
        &format!(
            "{{\"sessionId\":\"{parent_id}\",\"cwd\":\"/test/project\",\"message\":{{\"role\":\"user\",\"content\":\"Search the codebase\"}}}}\n"
        ),
    );

    let catalog = SessionCatalog::scan(home).expect("scan claude home");
    assert_eq!(catalog.sessions().len(), 1);
    assert_eq!(catalog.sessions()[0].id, parent_id);
    assert!(
        catalog.sessions()[0]
            .path
            .file_name()
            .is_some_and(|name| name == std::ffi::OsStr::new(&format!("{parent_id}.jsonl")))
    );
}

#[test]
fn distinct_logical_sessions_with_the_same_prefix_remain_ambiguous() {
    let temp = tempdir().expect("temp home");
    for session_id in ["codex-shared-prefix-a", "codex-shared-prefix-b"] {
        write(
            &temp
                .path()
                .join(format!(".codex/sessions/2026/07/31/{session_id}.jsonl")),
            &format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{session_id}\",\"cwd\":\"/work/mena\"}}}}\n"
            ),
        );
    }
    let catalog = SessionCatalog::scan(temp.path()).expect("scan distinct sessions");

    let error = catalog
        .resolve(Some("codex"), "codex-shared-prefix")
        .expect_err("a prefix shared by distinct logical sessions must stay ambiguous");

    assert!(format!("{error:#}").contains("selector is ambiguous"));
}

#[test]
fn indexes_supported_session_formats_and_real_usage_fields() {
    let temp = tempdir().expect("temp home");
    let home = temp.path();

    let codex = home.join(".codex/sessions/2026/01/02/codex.jsonl");
    write(
        &codex,
        concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"codex-id\",\"cwd\":\"/work/codex\",\"timestamp\":\"2026-01-02T03:04:05Z\"}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Fix Codex rendering\"}]}}\n",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"total_token_usage\":{\"total_tokens\":120}}}}\n"
        ),
    );

    let claude = home.join(".claude/projects/project/claude-id.jsonl");
    write(
        &claude,
        concat!(
            "{\"type\":\"user\",\"sessionId\":\"claude-id\",\"cwd\":\"/work/claude\",\"timestamp\":\"2026-01-02T03:04:05Z\"}\n",
            "{\"type\":\"user\",\"sessionId\":\"claude-id\",\"cwd\":\"/work/claude\",\"message\":{\"role\":\"user\",\"content\":\"Fix Claude rendering\"}}\n",
            "{\"type\":\"assistant\",\"sessionId\":\"claude-id\",\"message\":{\"usage\":{\"input_tokens\":10,\"output_tokens\":20,\"cache_read_input_tokens\":30,\"cache_creation_input_tokens\":40}}}\n"
        ),
    );

    let gemini_root = home.join(".gemini/tmp/gemini-project");
    write(&gemini_root.join(".project_root"), "/work/gemini\n");
    write(
        &gemini_root.join("chats/session.json"),
        r#"{"sessionId":"gemini-id","startTime":"2026-01-02T03:04:05Z","lastUpdated":"2026-01-02T03:05:05Z","messages":[{"type":"user","content":"Fix Gemini rendering"},{"type":"gemini","tokens":{"total":55}}]}"#,
    );

    write(
        &home.join(".local/share/opencode/storage/session/project/opencode.json"),
        r#"{"id":"opencode-id","directory":"/work/opencode","title":"Fix OpenCode rendering","time":{"created":1760000000000,"updated":1760000001000}}"#,
    );
    write(
        &home.join(".local/share/opencode/storage/message/opencode-id/message.json"),
        r#"{"role":"assistant","tokens":{"input":11,"output":12,"reasoning":13,"cache":{"read":14,"write":15}},"cost":0.25}"#,
    );

    write(
        &home.join(".pi/agent/sessions/project/pi.jsonl"),
        concat!(
            "{\"type\":\"session\",\"id\":\"pi-id\",\"cwd\":\"/work/pi\",\"timestamp\":\"2026-01-02T03:04:05Z\"}\n",
            "{\"type\":\"message\",\"message\":{\"role\":\"user\",\"content\":\"Fix Pi rendering\"}}\n",
            "{\"type\":\"message\",\"message\":{\"role\":\"assistant\",\"usage\":{\"totalTokens\":77,\"cost\":{\"total\":0.5}}}}\n"
        ),
    );

    write(
        &home.join(".omp/agent/sessions/project/omp.jsonl"),
        concat!(
            "{\"type\":\"title\",\"title\":\"Fix OMP rendering\"}\n",
            "{\"type\":\"session\",\"id\":\"omp-id\",\"cwd\":\"/work/omp\",\"timestamp\":\"2026-01-02T03:04:05Z\"}\n",
            "{\"type\":\"message\",\"message\":{\"role\":\"assistant\",\"usage\":{\"totalTokens\":88,\"cost\":{\"total\":0.75}}}}\n"
        ),
    );

    write_indexed_grok_fixture(home);

    let catalog = SessionCatalog::scan(home).expect("scan sessions");
    assert_indexed_provider_sessions(&catalog);
}

#[test]
fn codex_fork_rollout_keeps_the_parent_session_identity() {
    let temp = tempdir().expect("temp home");
    let home = temp.path();
    write(
        &home.join(".codex/sessions/2026/01/02/parent.jsonl"),
        "{\"type\":\"session_meta\",\"payload\":{\"id\":\"parent-id\",\"cwd\":\"/work\",\"timestamp\":\"2026-01-02T03:00:00Z\"}}\n",
    );
    // A forked rollout states its own meta first and re-states the parent
    // session's meta right after; the later record owns the identity, so
    // the fork file merges into the parent session instead of listing a
    // separate one.
    write(
        &home.join(".codex/sessions/2026/01/02/fork.jsonl"),
        concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"fork-id\",\"forked_from_id\":\"parent-id\",\"cwd\":\"/work\",\"timestamp\":\"2026-01-02T03:04:05Z\"}}\n",
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"parent-id\",\"cwd\":\"/work\",\"timestamp\":\"2026-01-02T03:00:00Z\"}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"Fix fork rendering\"}]}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"never reached\"}]}}\n",
        ),
    );
    let catalog = SessionCatalog::scan(home).expect("scan sessions");
    assert!(
        !catalog
            .sessions()
            .iter()
            .any(|session| session.id == "fork-id")
    );
    let parent = catalog
        .resolve(Some("codex"), "parent-id")
        .expect("parent session");
    assert_eq!(parent.title.as_deref(), Some("Fix fork rendering"));
    let mut native_files = vec![parent.path.clone()];
    native_files.extend(parent.related_paths.iter().cloned());
    assert_eq!(native_files.len(), 2);
}

#[test]
fn loads_every_codex_chat_message_without_truncating_content() {
    let temp = tempdir().expect("temp home");
    let session_path = temp.path().join(".codex/sessions/2026/01/02/codex.jsonl");
    let long_reply = "complete assistant response ".repeat(20);
    write(
        &session_path,
        &format!(
            "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"codex-detail\",\"cwd\":\"/work/codex\",\"timestamp\":\"2026-01-02T03:04:05Z\"}}}}\n\
                 {{\"type\":\"turn_context\",\"payload\":{{\"model\":\"gpt-5.5\"}}}}\n\
                 {{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"first question\"}}]}}}}\n\
                 {{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":{long_reply:?}}}]}}}}\n\
                 {{\"type\":\"turn_context\",\"payload\":{{\"model\":\"gpt-5.6\"}}}}\n\
                 {{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":\"second model answer\"}}]}}}}\n"
        ),
    );
    let catalog = SessionCatalog::scan(temp.path()).expect("scan sessions");
    let session = catalog
        .resolve(Some("codex"), "codex-detail")
        .expect("fixture session");

    let detail = catalog.detail(session).expect("load complete detail");

    assert_eq!(detail.messages.len(), 3);
    assert_eq!(detail.messages[0].kind, SessionMessageKind::User);
    assert_eq!(detail.messages[0].content, "first question");
    assert_eq!(detail.messages[1].kind, SessionMessageKind::Assistant);
    assert_eq!(detail.messages[1].content, long_reply);
    assert_eq!(detail.messages[1].model.as_deref(), Some("gpt-5.5"));
    assert_eq!(detail.messages[2].model.as_deref(), Some("gpt-5.6"));
}

#[test]
fn attaches_codex_turn_duration_and_last_request_tokens_to_the_assistant() {
    let temp = tempdir().expect("temp home");
    let session_path = temp.path().join(".codex/sessions/2026/01/02/metrics.jsonl");
    write(
        &session_path,
        concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"codex-metrics\",\"cwd\":\"/work\"}}\n",
            "{\"type\":\"turn_context\",\"payload\":{\"model\":\"gpt-5.6\"}}\n",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\"}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":\"question\"}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":\"answer\"}}\n",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"info\":{\"last_token_usage\":{\"input_tokens\":100,\"cached_input_tokens\":80,\"output_tokens\":23,\"reasoning_output_tokens\":5,\"total_tokens\":123},\"total_token_usage\":{\"total_tokens\":999}}}}\n",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_complete\",\"duration_ms\":6543,\"time_to_first_token_ms\":321}}\n"
        ),
    );
    let catalog = SessionCatalog::scan(temp.path()).expect("scan sessions");
    let session = catalog
        .resolve(Some("codex"), "codex-metrics")
        .expect("fixture session");

    let detail = catalog.detail(session).expect("load complete detail");
    let assistant = detail
        .messages
        .iter()
        .find(|message| message.kind == SessionMessageKind::Assistant)
        .expect("assistant message");

    assert_eq!(detail.session.tokens, Some(999));
    assert_eq!(assistant.model.as_deref(), Some("gpt-5.6"));
    let response = assistant
        .metrics
        .response
        .as_ref()
        .expect("response metrics");
    assert_eq!(response.tokens.total, Some(123));
    assert_eq!(response.tokens.input, Some(100));
    assert_eq!(response.tokens.output, Some(23));
    assert_eq!(response.tokens.cache_read, Some(80));
    assert_eq!(response.tokens.cache_write, None);
    assert_eq!(response.tokens.reasoning, Some(5));
    assert_eq!(response.duration_ms, Some(6_543));
    assert_eq!(response.time_to_first_token_ms, Some(321));
    assert_eq!(response.finish_reason.as_deref(), Some("task_complete"));
}

#[test]
fn classifies_codex_messages_tool_calls_results_and_errors() {
    let temp = tempdir().expect("temp home");
    let session_path = temp.path().join(".codex/sessions/2026/01/02/kinds.jsonl");
    write(
        &session_path,
        concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"codex-kinds\",\"cwd\":\"/work\"}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"question\"}]}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"read_file\",\"arguments\":\"{\\\"path\\\":\\\"README.md\\\"}\"}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"use_skill\",\"arguments\":\"{\\\"name\\\":\\\"frontend-design\\\"}\"}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"output\":\"file contents\"}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"answer\"}]}}\n",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"unknown_event\",\"value\":\"meta value\"}}\n",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"error\",\"message\":\"network failed\"}}\n"
        ),
    );
    let catalog = SessionCatalog::scan(temp.path()).expect("scan sessions");
    let session = catalog
        .resolve(Some("codex"), "codex-kinds")
        .expect("fixture session");

    let detail = catalog.detail(session).expect("load complete detail");

    assert_eq!(
        detail
            .messages
            .iter()
            .map(|message| message.kind)
            .collect::<Vec<_>>(),
        [
            SessionMessageKind::User,
            SessionMessageKind::ToolCall,
            SessionMessageKind::Skill,
            SessionMessageKind::ToolResult,
            SessionMessageKind::Assistant,
            SessionMessageKind::System,
            SessionMessageKind::Error,
        ]
    );
    assert!(detail.messages[1].content.contains("read_file"));
    assert!(detail.messages[2].content.contains("frontend-design"));
    assert!(detail.messages[3].content.contains("file contents"));
    assert!(detail.messages[5].content.contains("meta value"));
}

#[test]
fn correlates_codex_tool_end_metrics_and_aborted_turns() {
    let temp = tempdir().expect("temp home");
    let session_path = temp
        .path()
        .join(".codex/sessions/2026/01/02/native-metrics.jsonl");
    write(
        &session_path,
        concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"codex-native-metrics\",\"cwd\":\"/work\"}}\n",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\"}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"call_id\":\"call-1\",\"name\":\"read_file\",\"arguments\":\"{}\"}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"call-1\",\"output\":\"done\"}}\n",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"mcp_tool_call_end\",\"call_id\":\"call-1\",\"duration\":{\"secs\":1,\"nanos\":250000000},\"result\":{\"Ok\":{\"content\":\"done\"}}}}\n",
            "{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":\"partial answer\"}}\n",
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"turn_aborted\",\"reason\":\"user_interrupt\",\"duration_ms\":2345}}\n"
        ),
    );
    let catalog = SessionCatalog::scan(temp.path()).expect("scan sessions");
    let session = catalog
        .resolve(Some("codex"), "codex-native-metrics")
        .expect("fixture session");

    let detail = catalog.detail(session).expect("load complete detail");
    let tool = detail
        .messages
        .iter()
        .find(|message| message.kind == SessionMessageKind::ToolCall)
        .and_then(|message| message.metrics.tool.as_ref())
        .expect("correlated tool metrics");
    assert_eq!(tool.status.as_deref(), Some("completed"));
    assert_eq!(tool.duration_ms, Some(1_250));
    let response = detail
        .messages
        .iter()
        .find(|message| message.kind == SessionMessageKind::Assistant)
        .and_then(|message| message.metrics.response.as_ref())
        .expect("aborted response metrics");
    assert_eq!(response.duration_ms, Some(2_345));
    assert_eq!(response.finish_reason.as_deref(), Some("user_interrupt"));
}

#[test]
fn loads_every_claude_chat_message_in_file_order() {
    let temp = tempdir().expect("temp home");
    let session_path = temp
        .path()
        .join(".claude/projects/project/claude-detail.jsonl");
    write(
        &session_path,
        concat!(
            "{\"type\":\"user\",\"sessionId\":\"claude-detail\",\"cwd\":\"/work/claude\",\"timestamp\":\"2026-01-02T03:04:05Z\",\"message\":{\"role\":\"user\",\"content\":\"first question\"}}\n",
            "{\"type\":\"assistant\",\"sessionId\":\"claude-detail\",\"timestamp\":\"2026-01-02T03:04:06Z\",\"message\":{\"id\":\"message-1\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4-6\",\"usage\":{\"input_tokens\":10,\"output_tokens\":20,\"cache_read_input_tokens\":30,\"cache_creation_input_tokens\":40,\"cache_creation\":{\"ephemeral_5m_input_tokens\":15,\"ephemeral_1h_input_tokens\":25}},\"content\":[{\"type\":\"text\",\"text\":\"first answer\"}]}}\n",
            "{\"type\":\"user\",\"sessionId\":\"claude-detail\",\"timestamp\":\"2026-01-02T03:04:07Z\",\"message\":{\"role\":\"user\",\"content\":\"second question\"}}\n",
            "{\"type\":\"assistant\",\"sessionId\":\"claude-detail\",\"timestamp\":\"2026-01-02T03:04:08Z\",\"message\":{\"id\":\"message-2\",\"role\":\"assistant\",\"model\":\"claude-opus-4-6\",\"usage\":{\"input_tokens\":100,\"output_tokens\":50},\"content\":\"second answer\"}}\n"
        ),
    );
    let catalog = SessionCatalog::scan(temp.path()).expect("scan sessions");
    let session = catalog
        .resolve(Some("claude"), "claude-detail")
        .expect("fixture session");

    let detail = catalog.detail(session).expect("load complete detail");

    assert_eq!(
        detail
            .messages
            .iter()
            .map(|message| (message.kind, message.content.as_str()))
            .collect::<Vec<_>>(),
        [
            (SessionMessageKind::User, "first question"),
            (SessionMessageKind::Assistant, "first answer"),
            (SessionMessageKind::User, "second question"),
            (SessionMessageKind::Assistant, "second answer")
        ]
    );
    assert_eq!(
        detail.messages[1].model.as_deref(),
        Some("claude-sonnet-4-6")
    );
    assert_eq!(detail.messages[3].model.as_deref(), Some("claude-opus-4-6"));
    let first_response = detail.messages[1]
        .metrics
        .response
        .as_ref()
        .expect("first response metrics");
    assert_eq!(first_response.tokens.total, Some(100));
    assert_eq!(first_response.tokens.input, Some(10));
    assert_eq!(first_response.tokens.output, Some(20));
    assert_eq!(first_response.tokens.cache_read, Some(30));
    assert_eq!(first_response.tokens.cache_write, Some(40));
    assert_eq!(first_response.tokens.cache_write_5m, Some(15));
    assert_eq!(first_response.tokens.cache_write_1h, Some(25));
    assert_eq!(first_response.tokens.reasoning, None);
    let second_response = detail.messages[3]
        .metrics
        .response
        .as_ref()
        .expect("second response metrics");
    assert_eq!(second_response.tokens.total, Some(150));
    assert_eq!(second_response.tokens.input, Some(100));
    assert_eq!(second_response.tokens.output, Some(50));
}

#[test]
fn deduplicates_repeated_claude_usage_for_one_native_message() {
    let temp = tempdir().expect("temp home");
    let session_path = temp
        .path()
        .join(".claude/projects/project/claude-repeated.jsonl");
    write(
        &session_path,
        concat!(
            "{\"type\":\"user\",\"sessionId\":\"claude-repeated\",\"message\":{\"role\":\"user\",\"content\":\"question\"}}\n",
            "{\"type\":\"assistant\",\"sessionId\":\"claude-repeated\",\"message\":{\"id\":\"same-message\",\"role\":\"assistant\",\"usage\":{\"input_tokens\":10,\"output_tokens\":20},\"content\":\"partial\"}}\n",
            "{\"type\":\"assistant\",\"sessionId\":\"claude-repeated\",\"message\":{\"id\":\"same-message\",\"role\":\"assistant\",\"usage\":{\"input_tokens\":10,\"output_tokens\":20},\"content\":\"complete\"}}\n"
        ),
    );
    let catalog = SessionCatalog::scan(temp.path()).expect("scan sessions");
    let session = catalog
        .resolve(Some("claude"), "claude-repeated")
        .expect("fixture session");

    let detail = catalog.detail(session).expect("load complete detail");

    assert_eq!(detail.session.tokens, Some(30));
    assert!(detail.messages[1].metrics.response.is_none());
    assert_eq!(
        detail.messages[2]
            .metrics
            .response
            .as_ref()
            .expect("latest response metrics")
            .tokens
            .total,
        Some(30)
    );
}

#[test]
fn classifies_structured_claude_pi_and_omp_content_in_original_order() {
    let temp = tempdir().expect("temp home");
    for (root, session_id) in [
        (".claude/projects/project", "claude-structured"),
        (".pi/agent/sessions", "pi-structured"),
        (".omp/agent/sessions", "omp-structured"),
    ] {
        write(
            &temp.path().join(root).join(format!("{session_id}.jsonl")),
            &format!(
                "{{\"type\":\"session\",\"id\":\"{session_id}\",\"cwd\":\"/work\"}}\n\
                     {{\"type\":\"message\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"before\"}},{{\"type\":\"tool_use\",\"name\":\"read\",\"input\":{{\"path\":\"README.md\"}}}},{{\"type\":\"tool_result\",\"content\":\"file contents\"}},{{\"type\":\"thinking\",\"thinking\":\"private note\"}},{{\"type\":\"error\",\"message\":\"tool failed\"}},{{\"type\":\"tool_use\",\"name\":\"Skill\",\"input\":{{\"skill\":\"frontend-design\"}}}},{{\"type\":\"text\",\"text\":\"after\"}}]}}}}\n"
            ),
        );
    }
    let catalog = SessionCatalog::scan(temp.path()).expect("scan sessions");

    for (provider, session_id) in [
        ("claude", "claude-structured"),
        ("pi", "pi-structured"),
        ("omp", "omp-structured"),
    ] {
        let session = catalog
            .resolve(Some(provider), session_id)
            .expect("fixture session");
        let detail = catalog.detail(session).expect("load complete detail");
        assert_eq!(
            detail
                .messages
                .iter()
                .map(|message| message.kind)
                .collect::<Vec<_>>(),
            [
                SessionMessageKind::Assistant,
                SessionMessageKind::ToolCall,
                SessionMessageKind::ToolResult,
                SessionMessageKind::System,
                SessionMessageKind::Error,
                SessionMessageKind::Skill,
                SessionMessageKind::Assistant,
            ],
            "provider {provider}"
        );
        assert!(detail.messages[1].content.contains("README.md"));
        assert!(detail.messages[2].content.contains("file contents"));
        assert!(detail.messages[3].content.contains("private note"));
        assert!(detail.messages[4].content.contains("tool failed"));
        assert!(detail.messages[5].content.contains("frontend-design"));
    }
}

#[test]
fn loads_every_gemini_chat_message_from_the_session_document() {
    let temp = tempdir().expect("temp home");
    let session_path = temp
        .path()
        .join(".gemini/tmp/project/chats/gemini-detail.json");
    write(
        &session_path,
        r#"{"sessionId":"gemini-detail","messages":[{"type":"user","content":"first question"},{"type":"gemini","model":"gemini-3.1-pro","tokens":{"input":70,"output":15,"cached":11,"thoughts":5,"tool":6,"total":101},"durationMs":1234,"content":"first answer"},{"type":"user","content":"second question"},{"type":"gemini","model":"gemini-3.1-flash","tokens":{"input":150,"output":35,"cached":12,"thoughts":5,"tool":7,"total":202},"durationMs":2345,"content":"second answer"}]}"#,
    );
    let catalog = SessionCatalog::scan(temp.path()).expect("scan sessions");
    let session = catalog
        .resolve(Some("gemini"), "gemini-detail")
        .expect("fixture session");

    let detail = catalog.detail(session).expect("load complete detail");

    assert_eq!(
        detail
            .messages
            .iter()
            .map(|message| (message.kind, message.content.as_str()))
            .collect::<Vec<_>>(),
        [
            (SessionMessageKind::User, "first question"),
            (SessionMessageKind::Assistant, "first answer"),
            (SessionMessageKind::User, "second question"),
            (SessionMessageKind::Assistant, "second answer")
        ]
    );
    assert_eq!(detail.messages[1].model.as_deref(), Some("gemini-3.1-pro"));
    assert_eq!(
        detail.messages[3].model.as_deref(),
        Some("gemini-3.1-flash")
    );
    let first_response = detail.messages[1]
        .metrics
        .response
        .as_ref()
        .expect("first response metrics");
    assert_eq!(first_response.tokens.total, Some(101));
    assert_eq!(first_response.tokens.input, Some(70));
    assert_eq!(first_response.tokens.output, Some(15));
    assert_eq!(first_response.tokens.cache_read, Some(11));
    assert_eq!(first_response.tokens.reasoning, Some(5));
    assert_eq!(first_response.tokens.tool, Some(6));
    assert_eq!(first_response.duration_ms, Some(1_234));
    let second_response = detail.messages[3]
        .metrics
        .response
        .as_ref()
        .expect("second response metrics");
    assert_eq!(second_response.tokens.total, Some(202));
    assert_eq!(second_response.tokens.input, Some(150));
    assert_eq!(second_response.tokens.output, Some(35));
    assert_eq!(second_response.tokens.cache_read, Some(12));
    assert_eq!(second_response.tokens.reasoning, Some(5));
    assert_eq!(second_response.tokens.tool, Some(7));
    assert_eq!(second_response.duration_ms, Some(2_345));
    assert_eq!(detail.session.tokens, Some(303));
}

#[test]
fn classifies_structured_gemini_parts_without_collapsing_them() {
    let temp = tempdir().expect("temp home");
    let session_path = temp
        .path()
        .join(".gemini/tmp/project/chats/gemini-structured.json");
    write(
        &session_path,
        r#"{"sessionId":"gemini-structured","messages":[{"type":"gemini","content":[{"type":"text","text":"before"},{"type":"function_call","name":"read","args":{"path":"README.md"}},{"type":"function_call_output","output":"file contents"},{"type":"mystery","value":"meta"},{"type":"function_call","name":"activate_skill","args":{"skill":"frontend-design"}},{"type":"text","text":"after"}]}]}"#,
    );
    let catalog = SessionCatalog::scan(temp.path()).expect("scan sessions");
    let session = catalog
        .resolve(Some("gemini"), "gemini-structured")
        .expect("fixture session");

    let detail = catalog.detail(session).expect("load complete detail");

    assert_eq!(
        detail
            .messages
            .iter()
            .map(|message| message.kind)
            .collect::<Vec<_>>(),
        [
            SessionMessageKind::Assistant,
            SessionMessageKind::ToolCall,
            SessionMessageKind::ToolResult,
            SessionMessageKind::System,
            SessionMessageKind::Skill,
            SessionMessageKind::Assistant,
        ]
    );
    assert!(detail.messages[1].content.contains("README.md"));
    assert!(detail.messages[2].content.contains("file contents"));
    assert!(detail.messages[3].content.contains("meta"));
    assert!(detail.messages[4].content.contains("frontend-design"));
}

#[test]
fn loads_gemini_native_tool_calls_with_request_level_tool_tokens() {
    let temp = tempdir().expect("temp home");
    let session_path = temp
        .path()
        .join(".gemini/tmp/project/chats/gemini-tools.json");
    write(
        &session_path,
        r#"{"sessionId":"gemini-tools","messages":[{"type":"gemini","model":"gemini-3.1-pro","timestamp":"2026-01-01T00:00:00Z","tokens":{"input":10,"output":2,"tool":7,"total":19},"content":"","toolCalls":[{"id":"tool-1","name":"read_file","args":{"path":"README.md"},"result":{"content":"hello"},"status":"success","timestamp":"2026-01-01T00:00:00.500Z"}]}]}"#,
    );
    let catalog = SessionCatalog::scan(temp.path()).expect("scan sessions");
    let session = catalog
        .resolve(Some("gemini"), "gemini-tools")
        .expect("fixture session");

    let detail = catalog.detail(session).expect("load complete detail");
    assert_eq!(
        detail
            .messages
            .iter()
            .map(|message| message.kind)
            .collect::<Vec<_>>(),
        [SessionMessageKind::ToolCall, SessionMessageKind::ToolResult,]
    );
    let response = detail.messages[0]
        .metrics
        .response
        .as_ref()
        .expect("Gemini response metrics");
    assert_eq!(response.tokens.tool, Some(7));
    assert_eq!(detail.messages[0].model.as_deref(), Some("gemini-3.1-pro"));
    let tool = detail.messages[0]
        .metrics
        .tool
        .as_ref()
        .expect("Gemini tool status");
    assert_eq!(tool.status.as_deref(), Some("success"));
    assert!(detail.messages[0].content.contains("README.md"));
    assert!(detail.messages[1].content.contains("hello"));
}

#[test]
fn loads_pi_and_oh_my_pi_chat_messages_from_jsonl() {
    let temp = tempdir().expect("temp home");
    for (root, session_id) in [
        (".pi/agent/sessions", "pi-detail"),
        (".omp/agent/sessions", "omp-detail"),
    ] {
        write(
            &temp.path().join(root).join(format!("{session_id}.jsonl")),
            &format!(
                "{{\"type\":\"session\",\"id\":\"{session_id}\",\"cwd\":\"/work\"}}\n\
                     {{\"type\":\"message\",\"message\":{{\"role\":\"user\",\"content\":\"question for {session_id}\"}}}}\n\
                     {{\"type\":\"message\",\"message\":{{\"role\":\"assistant\",\"model\":\"model-for-{session_id}\",\"duration\":3456.4,\"usage\":{{\"input\":100,\"output\":89,\"cacheRead\":600,\"cacheWrite\":0,\"reasoningTokens\":12,\"totalTokens\":789,\"cost\":{{\"total\":0.125}}}},\"content\":\"answer for {session_id}\"}}}}\n"
            ),
        );
    }
    let catalog = SessionCatalog::scan(temp.path()).expect("scan sessions");

    for (provider, session_id) in [("pi", "pi-detail"), ("omp", "omp-detail")] {
        let session = catalog
            .resolve(Some(provider), session_id)
            .expect("fixture session");
        let detail = catalog.detail(session).expect("load complete detail");
        assert_eq!(detail.messages.len(), 2);
        assert_eq!(
            detail.messages[0].content,
            format!("question for {session_id}")
        );
        assert_eq!(
            detail.messages[1].content,
            format!("answer for {session_id}")
        );
        assert_eq!(
            detail.messages[1].model,
            Some(format!("model-for-{session_id}"))
        );
        let response = detail.messages[1]
            .metrics
            .response
            .as_ref()
            .expect("response metrics");
        assert_eq!(response.tokens.total, Some(789));
        assert_eq!(response.tokens.input, Some(100));
        assert_eq!(response.tokens.output, Some(89));
        assert_eq!(response.tokens.cache_read, Some(600));
        assert_eq!(response.tokens.cache_write, Some(0));
        assert_eq!(response.tokens.reasoning, Some(12));
        assert_eq!(response.duration_ms, Some(3_456));
        assert_eq!(detail.session.tokens, Some(789));
        assert_eq!(detail.session.cost_usd, Some(0.125));
    }
}

#[test]
fn loads_native_response_error_details_and_retry_count() {
    let temp = tempdir().expect("temp home");
    write(
        &temp
            .path()
            .join(".omp/agent/sessions/omp-response-error.jsonl"),
        concat!(
            "{\"type\":\"session\",\"id\":\"omp-response-error\",\"cwd\":\"/work\"}\n",
            "{\"type\":\"message\",\"message\":{\"role\":\"assistant\",\"model\":\"error-model\",\"stopReason\":\"error\",\"retryCount\":2,\"errorId\":\"rate_limit\",\"errorMessage\":\"retry budget exhausted\",\"usage\":{\"totalTokens\":12},\"content\":\"partial answer\"}}\n"
        ),
    );
    let catalog = SessionCatalog::scan(temp.path()).expect("scan sessions");
    let session = catalog
        .resolve(Some("omp"), "omp-response-error")
        .expect("fixture session");

    let detail = catalog.detail(session).expect("load complete detail");
    let response = detail.messages[0]
        .metrics
        .response
        .as_ref()
        .expect("response metrics");

    assert_eq!(response.finish_reason.as_deref(), Some("error"));
    assert_eq!(response.retry_count, Some(2));
    let error = response.error.as_ref().expect("error details");
    assert_eq!(error.code.as_deref(), Some("rate_limit"));
    assert_eq!(error.message, "retry budget exhausted");
}

#[test]
fn correlates_pi_family_tool_status_and_duration_by_native_call_id() {
    let temp = tempdir().expect("temp home");
    write(
        &temp
            .path()
            .join(".omp/agent/sessions/omp-tool-metrics.jsonl"),
        concat!(
            "{\"type\":\"session\",\"id\":\"omp-tool-metrics\",\"cwd\":\"/work\"}\n",
            "{\"type\":\"message\",\"message\":{\"role\":\"assistant\",\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"content\":[{\"type\":\"toolCall\",\"id\":\"call-1\",\"name\":\"bash\",\"arguments\":{\"command\":\"true\"}}]}}\n",
            "{\"type\":\"message\",\"message\":{\"role\":\"toolResult\",\"timestamp\":\"2026-01-01T00:00:00.450Z\",\"toolCallId\":\"call-1\",\"toolName\":\"bash\",\"isError\":false,\"details\":{\"wallTimeMs\":400},\"content\":\"done\"}}\n"
        ),
    );
    let catalog = SessionCatalog::scan(temp.path()).expect("scan sessions");
    let session = catalog
        .resolve(Some("omp"), "omp-tool-metrics")
        .expect("fixture session");

    let detail = catalog.detail(session).expect("load complete detail");

    assert_eq!(detail.messages[0].kind, SessionMessageKind::ToolCall);
    assert_eq!(detail.messages[1].kind, SessionMessageKind::ToolResult);
    let tool = detail.messages[0]
        .metrics
        .tool
        .as_ref()
        .expect("correlated tool metrics");
    assert_eq!(tool.status.as_deref(), Some("completed"));
    assert_eq!(tool.duration_ms, Some(400));
}

#[test]
fn correlates_claude_tool_errors_and_duration_by_native_call_id() {
    let temp = tempdir().expect("temp home");
    write(
        &temp
            .path()
            .join(".claude/projects/project/claude-tool-metrics.jsonl"),
        concat!(
            "{\"type\":\"assistant\",\"sessionId\":\"claude-tool-metrics\",\"timestamp\":\"2026-01-01T00:00:00.000Z\",\"message\":{\"role\":\"assistant\",\"model\":\"claude-sonnet\",\"stop_reason\":\"tool_use\",\"usage\":{\"input_tokens\":10,\"output_tokens\":1},\"content\":[{\"type\":\"tool_use\",\"id\":\"toolu-1\",\"name\":\"read\",\"input\":{\"path\":\"missing\"}}]}}\n",
            "{\"type\":\"user\",\"sessionId\":\"claude-tool-metrics\",\"timestamp\":\"2026-01-01T00:00:00.750Z\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"toolu-1\",\"is_error\":true,\"content\":\"file not found\"}]}}\n"
        ),
    );
    let catalog = SessionCatalog::scan(temp.path()).expect("scan sessions");
    let session = catalog
        .resolve(Some("claude"), "claude-tool-metrics")
        .expect("fixture session");

    let detail = catalog.detail(session).expect("load complete detail");
    let tool = detail.messages[0]
        .metrics
        .tool
        .as_ref()
        .expect("correlated tool metrics");

    assert_eq!(tool.status.as_deref(), Some("error"));
    assert_eq!(tool.duration_ms, Some(750));
    assert_eq!(
        tool.error.as_ref().map(|error| error.message.as_str()),
        Some("file not found")
    );
    assert_eq!(detail.messages[0].model.as_deref(), Some("claude-sonnet"));
    let response = detail.messages[0]
        .metrics
        .response
        .as_ref()
        .expect("tool-only model response metrics");
    assert_eq!(response.finish_reason.as_deref(), Some("tool_use"));
    assert_eq!(response.tokens.total, Some(11));
}

#[test]
fn loads_opencode_messages_and_all_of_their_parts_in_time_order() {
    let temp = tempdir().expect("temp home");
    let storage = temp.path().join(".local/share/opencode/storage");
    write(
        &storage.join("session/project/opencode-detail.json"),
        r#"{"id":"opencode-detail","directory":"/work/opencode","title":"Detail","time":{"created":1,"updated":4}}"#,
    );
    write(
        &storage.join("message/opencode-detail/message-2.json"),
        r#"{"id":"message-2","role":"assistant","modelID":"big-pickle","finish":"stop","time":{"created":3000,"completed":6450},"tokens":{"input":100,"output":20,"reasoning":10,"cache":{"read":5,"write":2}},"cost":0.42}"#,
    );
    write(
        &storage.join("part/message-2/part-2.json"),
        r#"{"type":"text","time":{"start":3400,"end":6400},"text":"second answer"}"#,
    );
    write(
        &storage.join("message/opencode-detail/message-1.json"),
        r#"{"id":"message-1","role":"user","time":{"created":2}}"#,
    );
    write(
        &storage.join("part/message-1/part-1.json"),
        r#"{"type":"text","text":"first question"}"#,
    );
    write(
        &storage.join("part/message-1/part-tool.json"),
        r#"{"type":"tool","tool":"read","state":{"status":"completed","time":{"start":100,"end":240},"input":{"path":"README.md"},"output":"file contents"}}"#,
    );
    write(
        &storage.join("part/message-1/part-skill.json"),
        r#"{"type":"tool","tool":"skill","state":{"input":{"name":"frontend-design"}}}"#,
    );
    let catalog = SessionCatalog::scan(temp.path()).expect("scan sessions");
    let session = catalog
        .resolve(Some("opencode"), "opencode-detail")
        .expect("fixture session");

    let detail = catalog.detail(session).expect("load complete detail");

    assert_eq!(detail.messages.len(), 5);
    assert_eq!(detail.messages[0].kind, SessionMessageKind::User);
    assert_eq!(detail.messages[0].content, "first question");
    assert_eq!(detail.messages[1].kind, SessionMessageKind::Skill);
    assert!(detail.messages[1].content.contains("frontend-design"));
    assert_eq!(detail.messages[2].kind, SessionMessageKind::ToolCall);
    assert!(detail.messages[2].content.contains("README.md"));
    let tool = detail.messages[2]
        .metrics
        .tool
        .as_ref()
        .expect("tool metrics");
    assert_eq!(tool.status.as_deref(), Some("completed"));
    assert_eq!(tool.duration_ms, Some(140));
    assert_eq!(detail.messages[3].kind, SessionMessageKind::ToolResult);
    assert!(detail.messages[3].content.contains("file contents"));
    assert_eq!(detail.messages[4].kind, SessionMessageKind::Assistant);
    assert_eq!(detail.messages[4].content, "second answer");
    assert_eq!(detail.messages[4].model.as_deref(), Some("big-pickle"));
    let response = detail.messages[4]
        .metrics
        .response
        .as_ref()
        .expect("native response metrics");
    assert_eq!(response.tokens.total, Some(137));
    assert_eq!(response.tokens.input, Some(100));
    assert_eq!(response.tokens.output, Some(20));
    assert_eq!(response.tokens.cache_read, Some(5));
    assert_eq!(response.tokens.cache_write, Some(2));
    assert_eq!(response.tokens.reasoning, Some(10));
    assert_eq!(response.duration_ms, Some(3_450));
    assert_eq!(response.time_to_first_token_ms, Some(400));
    assert_eq!(response.cost_usd, Some(0.42));
    assert_eq!(response.finish_reason.as_deref(), Some("stop"));
    assert_eq!(detail.session.tokens, Some(137));
    assert_eq!(detail.session.cost_usd, Some(0.42));
}

#[test]
fn permanently_deletes_an_opencode_session_and_all_of_its_artifacts() {
    let temp = tempdir().expect("temp home");
    let home = temp.path();
    let session_path =
        home.join(".local/share/opencode/storage/session/project/opencode-delete.json");
    write(
        &session_path,
        r#"{"id":"opencode-delete","directory":"/work/opencode","title":"Delete me","time":{"created":1760000000000,"updated":1760000001000}}"#,
    );
    write(
        &home.join(".local/share/opencode/storage/message/opencode-delete/message-delete.json"),
        r#"{"id":"message-delete","role":"assistant"}"#,
    );
    write(
        &home.join(".local/share/opencode/storage/part/message-delete/part-delete.json"),
        "{}",
    );
    write(
        &home.join(".local/share/opencode/storage/session_diff/opencode-delete.json"),
        "{}",
    );
    write(
        &home.join(".local/share/opencode/storage/todo/opencode-delete.json"),
        "{}",
    );
    let decoy = home.join(".local/share/opencode/storage/todo/keep.json");
    write(&decoy, "{}");

    let catalog = SessionCatalog::scan(home).expect("scan sessions");
    let session = catalog
        .resolve(Some("opencode"), "opencode-delete")
        .expect("session to delete");
    let result = catalog
        .delete_session(session)
        .expect("delete session artifacts");

    assert!(result.files >= 4);
    assert!(!session_path.exists());
    assert!(
        !home
            .join(".local/share/opencode/storage/message/opencode-delete")
            .exists()
    );
    assert!(
        !home
            .join(".local/share/opencode/storage/part/message-delete")
            .exists()
    );
    assert!(decoy.exists(), "unrelated provider data must be preserved");
}

#[test]
fn deletion_rejects_session_ids_that_can_escape_a_provider_store() {
    let temp = tempdir().expect("temp home");
    let session_path = temp
        .path()
        .join(".local/share/opencode/storage/session/project/unsafe.json");
    write(
        &session_path,
        r#"{"id":"../../outside","directory":"/work/opencode","title":"Unsafe"}"#,
    );
    let catalog = SessionCatalog::scan(temp.path()).expect("scan sessions");
    let session = catalog.sessions().first().expect("unsafe fixture session");

    let error = catalog
        .delete_session(session)
        .expect_err("unsafe session ID must be rejected");

    assert!(format!("{error:#}").contains("unsafe session ID"));
    assert!(session_path.exists());
}

#[cfg(unix)]
#[test]
fn deletion_rejects_nested_symlinks_that_escape_a_provider_store() {
    let temp = tempdir().expect("temp home");
    let home = temp.path();
    let session_id = "claude-safe-id";
    let session_path = home.join(format!(".claude/projects/project/{session_id}.jsonl"));
    write(
        &session_path,
        &format!("{{\"sessionId\":\"{session_id}\",\"cwd\":\"/work\"}}\n"),
    );
    let outside = home.join("outside-file-history");
    let escaped = outside.join(session_id).join("change.txt");
    write(&escaped, "must survive");
    symlink(&outside, home.join(".claude/file-history")).expect("create nested symlink");
    let catalog = SessionCatalog::scan(home).expect("scan sessions");
    let session = catalog
        .resolve(Some("claude"), session_id)
        .expect("fixture session");

    let error = catalog
        .delete_session(session)
        .expect_err("escaped artifact must be rejected");

    assert!(format!("{error:#}").contains("outside its provider store"));
    assert!(session_path.exists());
    assert!(escaped.exists());
}

#[test]
fn permanently_deletes_claude_session_history_team_and_runtime_artifacts() {
    let temp = tempdir().expect("temp home");
    let home = temp.path();
    let session_id = "2672f407-038c-409d-8aa0-e8e7c39cf8d7";
    let session_path = home.join(format!(".claude/projects/project/{session_id}.jsonl"));
    write(
        &session_path,
        &format!(
            "{{\"sessionId\":\"{session_id}\",\"cwd\":\"/work/claude\",\"message\":{{\"role\":\"user\",\"content\":\"Delete me\"}}}}\n"
        ),
    );
    let history = home.join(".claude/history.jsonl");
    write(
        &history,
        &format!(
            "{{\"sessionId\":\"{session_id}\",\"display\":\"delete\"}}\n{{\"sessionId\":\"keep\",\"display\":\"keep\"}}\n"
        ),
    );
    let runtime = home.join(".claude/sessions/1.json");
    write(&runtime, &format!("{{\"sessionId\":\"{session_id}\"}}"));
    let runtime_decoy = home.join(".claude/sessions/2.json");
    write(&runtime_decoy, r#"{"sessionId":"keep"}"#);
    let team = home.join(".claude/teams/session-2672f407");
    write(
        &team.join("config.json"),
        &format!("{{\"leadSessionId\":\"{session_id}\"}}"),
    );
    let tasks = home.join(".claude/tasks/session-2672f407");
    write(&tasks.join("agent-id.txt"), "task");
    let debug = home.join(".claude/debug/agent-id.txt");
    write(&debug, "debug");
    let cache = home.join(".claude/plugins/claude-hud/transcript-cache/delete.json");
    write(
        &cache,
        &serde_json::json!({"transcriptPath": session_path}).to_string(),
    );
    let cache_decoy = home.join(".claude/plugins/claude-hud/transcript-cache/keep.json");
    write(&cache_decoy, r#"{"transcriptPath":"/tmp/keep.jsonl"}"#);
    write(
        &home.join(format!(".claude/file-history/{session_id}/change.txt")),
        "history",
    );

    let catalog = SessionCatalog::scan(home).expect("scan sessions");
    let session = catalog
        .resolve(Some("claude"), session_id)
        .expect("session to delete");
    let result = catalog
        .delete_session(session)
        .expect("delete Claude session");

    assert_eq!(result.index_records, 1);
    assert!(!session_path.exists());
    assert!(!runtime.exists());
    assert!(!team.exists());
    assert!(!tasks.exists());
    assert!(!debug.exists());
    assert!(!cache.exists());
    assert!(runtime_decoy.exists());
    assert!(cache_decoy.exists());
    assert_eq!(
        fs::read_to_string(history).expect("read rewritten history"),
        "{\"sessionId\":\"keep\",\"display\":\"keep\"}\n"
    );
}

#[test]
fn permanently_deletes_codex_session_indexes_and_shell_snapshots() {
    let temp = tempdir().expect("temp home");
    let home = temp.path();
    let session_path = home.join(".codex/sessions/2026/01/02/delete.jsonl");
    write(
        &session_path,
        "{\"type\":\"session_meta\",\"payload\":{\"id\":\"codex-delete\",\"cwd\":\"/work/codex\"}}\n",
    );
    let snapshot = home.join(".codex/shell_snapshots/codex-delete.1.sh");
    write(&snapshot, "snapshot");
    let decoy = home.join(".codex/shell_snapshots/codex-keep.1.sh");
    write(&decoy, "snapshot");
    let session_index = home.join(".codex/session_index.jsonl");
    write(
        &session_index,
        concat!(
            "{\"id\":\"codex-delete\",\"thread_name\":\"Delete me\"}\n",
            "{\"id\":\"codex-keep\",\"thread_name\":\"Keep me\"}\n"
        ),
    );
    let database_path = home.join(".codex/state_5.sqlite");
    let connection = rusqlite::Connection::open(&database_path).expect("open fixture db");
    connection
        .execute_batch(
            "CREATE TABLE threads (id TEXT PRIMARY KEY, title TEXT NOT NULL);\
                 CREATE TABLE thread_dynamic_tools (thread_id TEXT NOT NULL);\
                 CREATE TABLE thread_spawn_edges (parent_thread_id TEXT, child_thread_id TEXT);\
                 INSERT INTO threads VALUES ('codex-delete', 'Delete me');\
                 INSERT INTO thread_dynamic_tools VALUES ('codex-delete');\
                 INSERT INTO thread_spawn_edges VALUES ('codex-delete', 'child');",
        )
        .expect("seed fixture db");
    drop(connection);

    let catalog = SessionCatalog::scan(home).expect("scan sessions");
    let session = catalog
        .resolve(Some("codex"), "codex-delete")
        .expect("session to delete");
    assert_eq!(session.title.as_deref(), Some("Delete me"));
    let result = catalog
        .delete_session(session)
        .expect("delete Codex session");

    assert!(result.index_records >= 4);
    assert!(!session_path.exists());
    assert!(!snapshot.exists());
    assert!(decoy.exists());
    assert_eq!(
        fs::read_to_string(session_index).expect("read rewritten session index"),
        "{\"id\":\"codex-keep\",\"thread_name\":\"Keep me\"}\n"
    );
    let connection = rusqlite::Connection::open(database_path).expect("reopen fixture db");
    let remaining: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM threads WHERE id = 'codex-delete'",
            [],
            |row| row.get(0),
        )
        .expect("query fixture db");
    assert_eq!(remaining, 0);
}

#[test]
fn records_a_resume_selector_without_claiming_it_is_still_current() {
    let temp = tempdir().expect("temp home");
    write(
        &temp.path().join(".codex/sessions/2026/01/02/codex.jsonl"),
        "{\"type\":\"session_meta\",\"payload\":{\"id\":\"codex-id\",\"cwd\":\"/work/project\"}}\n",
    );
    write(
        &temp.path().join(".codex/sessions/2026/01/02/other.jsonl"),
        "{\"type\":\"session_meta\",\"payload\":{\"id\":\"codex-other\",\"cwd\":\"/work/project\"}}\n",
    );
    let catalog = SessionCatalog::scan(temp.path()).expect("scan sessions");
    let agent = live_agent(
        AgentKind::Codex,
        42,
        &["codex", "resume", "codex-id"],
        "/work/project",
        100,
    );

    assert_eq!(
        catalog
            .association_for_process(&agent)
            .expect("associate process"),
        AssociationDecision::ProtectProvider
    );
    let protected = catalog
        .protection(std::slice::from_ref(&agent))
        .expect("protect sessions");
    assert!(protected.exact_active_targets.is_empty());
    assert_eq!(
        protected.protected_targets,
        std::collections::BTreeSet::from([
            "codex:codex-id".to_owned(),
            "codex:codex-other".to_owned(),
        ])
    );
}

#[test]
fn never_infers_a_live_session_from_recency_or_shared_project() {
    let temp = tempdir().expect("temp home");
    for id in ["older", "newer"] {
        write(
            &temp
                .path()
                .join(format!(".codex/sessions/2026/01/02/{id}.jsonl")),
            &format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"/work/project\"}}}}\n"
            ),
        );
    }
    let catalog = SessionCatalog::scan(temp.path()).expect("scan sessions");
    let agent = live_agent(
        AgentKind::Codex,
        42,
        &["codex", "app-server"],
        "/work/project",
        1,
    );

    assert_eq!(
        catalog
            .association_for_process(&agent)
            .expect("associate process"),
        AssociationDecision::ProtectProvider
    );
    let protected = catalog
        .protection(std::slice::from_ref(&agent))
        .expect("protect sessions");
    assert!(protected.exact_active_targets.is_empty());
    assert_eq!(
        protected.protected_targets,
        std::collections::BTreeSet::from(["codex:newer".to_owned(), "codex:older".to_owned(),])
    );
}

#[test]
fn claude_runtime_identity_requires_pid_start_time_and_project() {
    let temp = tempdir().expect("temp home");
    write(
        &temp
            .path()
            .join(".claude/projects/-work-project/claude-id.jsonl"),
        "{\"sessionId\":\"claude-id\",\"cwd\":\"/work/project\"}\n",
    );
    write(
        &temp
            .path()
            .join(".claude/projects/-work-project/claude-other.jsonl"),
        "{\"sessionId\":\"claude-other\",\"cwd\":\"/work/project\"}\n",
    );
    write(
        &temp.path().join(".claude/sessions/42.json"),
        "{\"pid\":42,\"sessionId\":\"claude-id\",\"cwd\":\"/work/project\",\"startedAt\":101000}\n",
    );
    let catalog = SessionCatalog::scan(temp.path()).expect("scan sessions");
    let agent = live_agent(AgentKind::ClaudeCode, 42, &["claude"], "/work/project", 100);

    let AssociationDecision::Exact { session, evidence } = catalog
        .association_for_process(&agent)
        .expect("associate process")
    else {
        panic!("native runtime must identify one exact session");
    };
    assert_eq!(session.id, "claude-id");
    assert_eq!(evidence, AssociationEvidence::NativeRuntime);
    let protected = catalog
        .protection(std::slice::from_ref(&agent))
        .expect("protect exact session");
    assert_eq!(
        protected.exact_active_targets,
        std::collections::BTreeSet::from(["claude:claude-id".to_owned()])
    );
    assert_eq!(
        protected.protected_targets,
        std::collections::BTreeSet::from(["claude:claude-id".to_owned()])
    );

    let stale_agent = LiveAgent {
        process: ProcessSnapshot {
            started_at: 200,
            ..agent.process
        },
        ..agent
    };
    assert_eq!(
        catalog
            .association_for_process(&stale_agent)
            .expect("reject stale runtime identity"),
        AssociationDecision::ProtectProvider
    );
    assert_eq!(
        catalog
            .protection(std::slice::from_ref(&stale_agent))
            .expect("protect provider sessions")
            .protected_targets
            .len(),
        2
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn pi_family_runtime_identity_uses_the_open_native_transcript() {
    let temp = tempdir().expect("temp home");
    let session_path = temp
        .path()
        .join(".omp/agent/sessions/project/session.jsonl");
    write(
        &session_path,
        "{\"type\":\"session\",\"id\":\"omp-id\",\"cwd\":\"/work/project\"}\n",
    );
    let _open_transcript = fs::File::open(&session_path).expect("hold transcript open");
    let catalog = SessionCatalog::scan(temp.path()).expect("scan sessions");
    let agent = live_agent(
        AgentKind::OhMyPi,
        std::process::id(),
        &["omp"],
        "/work/project",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("current time")
            .as_secs(),
    );

    let AssociationDecision::Exact { session, evidence } = catalog
        .association_for_process(&agent)
        .expect("associate process")
    else {
        panic!("open transcript must identify one exact session");
    };
    assert_eq!(session.id, "omp-id");
    assert_eq!(evidence, AssociationEvidence::OpenSessionFile);
}

#[test]
fn conflicting_native_selectors_are_ambiguous_and_fail_closed() {
    let temp = tempdir().expect("temp home");
    for id in ["gemini-a", "gemini-b"] {
        write(
            &temp
                .path()
                .join(format!(".gemini/tmp/project/chats/{id}.json")),
            &format!("{{\"sessionId\":\"{id}\",\"messages\":[]}}"),
        );
    }
    write(
        &temp.path().join(".gemini/tmp/project/.project_root"),
        "/work/project\n",
    );
    let catalog = SessionCatalog::scan(temp.path()).expect("scan sessions");
    let agent = live_agent(
        AgentKind::GeminiCli,
        42,
        &["gemini", "--resume", "gemini-a", "--resume", "gemini-b"],
        "/work/project",
        100,
    );

    assert_eq!(
        catalog
            .association_for_process(&agent)
            .expect("associate process"),
        AssociationDecision::ProtectProvider
    );
    assert_eq!(
        catalog
            .protection(std::slice::from_ref(&agent))
            .expect("protect ambiguous sessions")
            .protected_targets
            .len(),
        2
    );
}

#[test]
fn grok_detail_skips_bad_lines_and_does_not_convert_cost_ticks() {
    let temp = tempdir().expect("temp home");
    let home = temp.path();
    let id = "01a05b12-60ae-7790-8716-9293782180a9";
    write_grok_session(
        home,
        "%2Fwork%2Fproject",
        id,
        &format!(r#"{{"info":{{"id":"{id}","cwd":"/work/project"}},"generated_title":"Ticks"}}"#),
        concat!(
            "not-json\n",
            r#"{"timestamp":1767323045,"method":"session/update","params":{"sessionId":"01a05b12-60ae-7790-8716-9293782180a9","update":{"sessionUpdate":"hook_execution","event_name":"user_prompt_submit"}}}"#,
            "\n",
            r#"{"timestamp":1767323046,"method":"session/update","params":{"sessionId":"01a05b12-60ae-7790-8716-9293782180a9","update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"Hello "}}}}"#,
            "\n",
            r#"{"timestamp":1767323046,"method":"session/update","params":{"sessionId":"01a05b12-60ae-7790-8716-9293782180a9","update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"Grok"}}}}"#,
            "\n",
            r#"{"timestamp":1767323047,"method":"session/update","params":{"sessionId":"01a05b12-60ae-7790-8716-9293782180a9","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Hi."},"_meta":{"modelId":"grok-4.6"}}}}"#,
            "\n",
            r#"{"timestamp":1767323048,"method":"session/update","params":{"sessionId":"01a05b12-60ae-7790-8716-9293782180a9","update":{"sessionUpdate":"turn_completed","stop_reason":"end_turn","elapsed_ms":40,"usage":{"inputTokens":11,"outputTokens":2,"totalTokens":13,"costUsdTicks":8800}}}}"#,
            "\n",
        ),
    );
    let catalog = SessionCatalog::scan(home).expect("scan grok session");
    let session = catalog.resolve(Some("grok"), id).expect("grok session");
    let detail = catalog.detail(session).expect("load grok detail");
    assert_eq!(detail.messages.len(), 2);
    assert_eq!(detail.messages[0].kind, SessionMessageKind::User);
    assert_eq!(detail.messages[0].content, "Hello Grok");
    assert_eq!(detail.messages[1].kind, SessionMessageKind::Assistant);
    assert_eq!(detail.messages[1].content, "Hi.");
    assert_eq!(detail.session.tokens, Some(13));
    assert_eq!(detail.session.cost_usd, None);
    assert_eq!(
        detail.messages[1]
            .metrics
            .response
            .as_ref()
            .and_then(|metrics| metrics.cost_usd),
        None
    );
}

#[test]
fn grok_catalog_decodes_project_paths_and_ignores_index_noise() {
    let temp = tempdir().expect("temp home");
    let home = temp.path();
    write_grok_session(
        home,
        "%2Fwork%2Fproject",
        "01a05b12-60ae-7790-8716-9293782180a9",
        r#"{"info":{"id":"01a05b12-60ae-7790-8716-9293782180a9","cwd":"/work/project"},"generated_title":"First","created_at":"2026-01-02T03:04:05Z","updated_at":"2026-01-02T03:05:05Z"}"#,
        grok_user_turn("01a05b12-60ae-7790-8716-9293782180a9", "hello"),
    );
    write_grok_session(
        home,
        "%2Fwork%2Fproject",
        "01a05b0f-7770-7cd1-896a-f144f338b84b",
        r#"{"info":{"id":"01a05b0f-7770-7cd1-896a-f144f338b84b","cwd":"/work/project"},"generated_title":"Second","created_at":"2026-01-02T03:06:05Z","updated_at":"2026-01-02T03:07:05Z"}"#,
        grok_user_turn("01a05b0f-7770-7cd1-896a-f144f338b84b", "again"),
    );
    write(
        &home.join(".grok/sessions/%2Fwork%2Fproject/prompt_history.jsonl"),
        "{\"prompt\":\"ignore me\"}\n",
    );
    write(
        &home.join(".grok/sessions/session_search.sqlite"),
        "not-a-session",
    );
    write(
        &home.join(".grok/sessions/%2Fwork%2Fproject/empty-draft/summary.json"),
        r#"{"info":{"id":"empty-draft","cwd":"/work/project"},"session_summary":"","num_messages":0}"#,
    );
    let nested = home.join(
        ".grok/sessions/%2Fwork%2Fproject/01a05b12-60ae-7790-8716-9293782180a9/subagents/child",
    );
    write(&nested.join("meta.json"), r#"{"id":"child"}"#);

    let catalog = SessionCatalog::scan(home).expect("scan grok sessions");
    let grok: Vec<_> = catalog
        .sessions()
        .iter()
        .filter(|session| session.kind == AgentKind::Grok)
        .collect();
    assert_eq!(grok.len(), 2);
    assert!(
        grok.iter().all(
            |session| session.project.as_deref() == Some(std::path::Path::new("/work/project"))
        )
    );
    assert!(grok.iter().all(|session| session.id != "empty-draft"));
    assert!(grok.iter().all(|session| session.id != "child"));
    assert!(
        grok.iter()
            .any(|session| session.id == "01a05b12-60ae-7790-8716-9293782180a9")
    );
    assert!(
        grok.iter()
            .any(|session| session.id == "01a05b0f-7770-7cd1-896a-f144f338b84b")
    );

    let expanded = SessionCatalog::scan_provider(home, Some(&AgentKind::Grok), true, false)
        .expect("scan grok drafts");
    assert!(
        expanded
            .sessions()
            .iter()
            .any(|session| session.id == "empty-draft")
    );
}

#[test]
fn grok_long_group_directory_reads_cwd_marker() {
    let temp = tempdir().expect("temp home");
    let home = temp.path();
    let group = home.join(".grok/sessions/very-long-slug-without-percent-abcdef");
    write(&group.join(".cwd"), "/work/very/long/project\n");
    write_grok_session_at(
        &group.join("01a05b62-b578-7412-8664-94f9ce05a3cd"),
        r#"{"info":{"id":"01a05b62-b578-7412-8664-94f9ce05a3cd"},"generated_title":"Long path"}"#,
        grok_user_turn("01a05b62-b578-7412-8664-94f9ce05a3cd", "long"),
    );

    let catalog = SessionCatalog::scan(home).expect("scan long grok group");
    let session = catalog
        .resolve(Some("grok"), "01a05b62-b578-7412-8664-94f9ce05a3cd")
        .expect("long grok session");
    assert_eq!(
        session.project.as_deref(),
        Some(std::path::Path::new("/work/very/long/project"))
    );
}

#[test]
fn grok_home_env_overrides_os_home_only_for_grok() {
    let temp = tempdir().expect("os home");
    let grok_root = tempdir().expect("grok home");
    let home = temp.path();
    write(
        &home.join(".claude/projects/project/claude-id.jsonl"),
        "{\"sessionId\":\"claude-id\",\"cwd\":\"/work/claude\",\"message\":{\"role\":\"user\",\"content\":\"Claude stays\"}}\n",
    );
    write_grok_session_in(
        grok_root.path(),
        "%2Fwork%2Foverride",
        "grok-override",
        r#"{"info":{"id":"grok-override","cwd":"/work/override"},"generated_title":"Override"}"#,
        grok_user_turn("grok-override", "override"),
    );
    write_grok_session(
        home,
        "%2Fwork%2Fdecoy",
        "grok-decoy",
        r#"{"info":{"id":"grok-decoy","cwd":"/work/decoy"},"generated_title":"Decoy"}"#,
        grok_user_turn("grok-decoy", "decoy"),
    );

    super::set_test_grok_home(Some(grok_root.path().to_path_buf()));
    let _guard = super::TestGrokHomeGuard;
    let catalog =
        SessionCatalog::scan_provider(home, None, false, false).expect("scan with GROK_HOME");

    assert!(
        catalog
            .sessions()
            .iter()
            .any(|session| session.kind == AgentKind::ClaudeCode && session.id == "claude-id")
    );
    assert!(
        catalog
            .sessions()
            .iter()
            .any(|session| session.kind == AgentKind::Grok && session.id == "grok-override")
    );
    assert!(
        catalog
            .sessions()
            .iter()
            .all(|session| session.kind != AgentKind::Grok || session.id != "grok-decoy")
    );
}

#[test]
fn grok_deletes_only_the_selected_session_directory() {
    let temp = tempdir().expect("temp home");
    let home = temp.path();
    let target_id = "01a05b12-60ae-7790-8716-9293782180a9";
    let neighbor_id = "01a05b0f-7770-7cd1-896a-f144f338b84b";
    write_grok_session(
        home,
        "%2Fwork%2Fproject",
        target_id,
        &format!(
            r#"{{"info":{{"id":"{target_id}","cwd":"/work/project"}},"generated_title":"Delete me"}}"#
        ),
        grok_user_turn(target_id, "delete"),
    );
    write_grok_session(
        home,
        "%2Fwork%2Fproject",
        neighbor_id,
        &format!(
            r#"{{"info":{{"id":"{neighbor_id}","cwd":"/work/project"}},"generated_title":"Keep me"}}"#
        ),
        grok_user_turn(neighbor_id, "keep"),
    );
    let worktree = home.join(".grok/worktrees/keep");
    write(&worktree.join("file.txt"), "untouched");
    write(&home.join(".grok/worktrees.db"), "sqlite");
    write(
        &home.join(".grok/sessions/session_search.sqlite"),
        "search-index",
    );
    write(
        &home.join(".grok/sessions/%2Fwork%2Fproject/prompt_history.jsonl"),
        "{}\n",
    );

    let catalog = SessionCatalog::scan(home).expect("scan grok sessions");
    let session = catalog
        .resolve(Some("grok"), target_id)
        .expect("target session")
        .clone();
    catalog
        .delete_session(&session)
        .expect("delete grok session");

    assert!(!session.path.exists());
    assert!(!session.path.parent().expect("session dir").exists());
    assert!(
        home.join(format!(
            ".grok/sessions/%2Fwork%2Fproject/{neighbor_id}/updates.jsonl"
        ))
        .exists()
    );
    assert!(worktree.join("file.txt").exists());
    assert!(home.join(".grok/worktrees.db").exists());
    assert!(home.join(".grok/sessions/session_search.sqlite").exists());
    assert!(
        home.join(".grok/sessions/%2Fwork%2Fproject/prompt_history.jsonl")
            .exists()
    );
}

#[test]
fn grok_runtime_metadata_marks_the_matching_session_active() {
    let temp = tempdir().expect("temp home");
    let home = temp.path();
    let session_id = "01a05b62-b578-7412-8664-94f9ce05a3cd";
    write_grok_session(
        home,
        "%2Fwork%2Fproject",
        session_id,
        &format!(
            r#"{{"info":{{"id":"{session_id}","cwd":"/work/project"}},"generated_title":"Active"}}"#
        ),
        grok_user_turn(session_id, "active"),
    );
    write_grok_session(
        home,
        "%2Fwork%2Fproject",
        "01a05b0f-7770-7cd1-896a-f144f338b84b",
        r#"{"info":{"id":"01a05b0f-7770-7cd1-896a-f144f338b84b","cwd":"/work/project"},"generated_title":"Other"}"#,
        grok_user_turn("01a05b0f-7770-7cd1-896a-f144f338b84b", "other"),
    );
    write(
        &home.join(".grok/active_sessions.json"),
        &format!(
            r#"[{{"session_id":"{session_id}","pid":42,"cwd":"/work/project","opened_at":"2026-01-02T03:04:05Z"}}]"#
        ),
    );

    let catalog = SessionCatalog::scan(home).expect("scan grok sessions");
    let agent = live_agent(AgentKind::Grok, 42, &["grok"], "/work/project", 100);
    let AssociationDecision::Exact { session, evidence } = catalog
        .association_for_process(&agent)
        .expect("associate grok process")
    else {
        panic!("active_sessions.json must identify one exact session");
    };
    assert_eq!(session.id, session_id);
    assert_eq!(evidence, AssociationEvidence::NativeRuntime);
    let protected = catalog
        .protection(std::slice::from_ref(&agent))
        .expect("protect exact grok session");
    assert_eq!(
        protected.exact_active_targets,
        std::collections::BTreeSet::from([format!("grok:{session_id}")])
    );
}

#[test]
fn grok_does_not_mark_active_from_shared_project_alone() {
    let temp = tempdir().expect("temp home");
    let home = temp.path();
    write_grok_session(
        home,
        "%2Fwork%2Fproject",
        "01a05b12-60ae-7790-8716-9293782180a9",
        r#"{"info":{"id":"01a05b12-60ae-7790-8716-9293782180a9","cwd":"/work/project"},"generated_title":"Project"}"#,
        grok_user_turn("01a05b12-60ae-7790-8716-9293782180a9", "project"),
    );
    let catalog = SessionCatalog::scan(home).expect("scan grok sessions");
    let agent = live_agent(AgentKind::Grok, 42, &["grok"], "/work/project", 100);
    assert_eq!(
        catalog
            .association_for_process(&agent)
            .expect("associate grok process"),
        AssociationDecision::ProtectProvider
    );
    let protected = catalog
        .protection(std::slice::from_ref(&agent))
        .expect("protect grok catalog");
    assert!(protected.exact_active_targets.is_empty());
    assert_eq!(
        protected.protected_targets,
        std::collections::BTreeSet::from(["grok:01a05b12-60ae-7790-8716-9293782180a9".to_owned()])
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn grok_open_session_directory_file_is_exact_active_evidence() {
    let temp = tempdir().expect("temp home");
    let home = temp.path();
    let session_id = "01a05b12-60ae-7790-8716-9293782180a9";
    let updates = write_grok_session(
        home,
        "%2Fwork%2Fproject",
        session_id,
        &format!(
            r#"{{"info":{{"id":"{session_id}","cwd":"/work/project"}},"generated_title":"Open file"}}"#
        ),
        grok_user_turn(session_id, "open"),
    );
    let events = updates.parent().expect("session dir").join("events.jsonl");
    write(&events, "{}\n");
    let _open_events = fs::File::open(&events).expect("hold events open");
    let catalog = SessionCatalog::scan(home).expect("scan grok sessions");
    let agent = live_agent(
        AgentKind::Grok,
        std::process::id(),
        &["grok"],
        "/work/project",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("current time")
            .as_secs(),
    );

    let AssociationDecision::Exact { session, evidence } = catalog
        .association_for_process(&agent)
        .expect("associate grok process")
    else {
        panic!("an open file in the session directory must identify one exact session");
    };
    assert_eq!(session.id, session_id);
    assert_eq!(evidence, AssociationEvidence::OpenSessionFile);
}

#[cfg(unix)]
#[test]
fn grok_deletion_unlinks_internal_symlinks_without_following_them() {
    let temp = tempdir().expect("temp home");
    let home = temp.path();
    let session_id = "01a05b12-60ae-7790-8716-9293782180a9";
    write_grok_session(
        home,
        "%2Fwork%2Fproject",
        session_id,
        &format!(
            r#"{{"info":{{"id":"{session_id}","cwd":"/work/project"}},"generated_title":"Escape"}}"#
        ),
        grok_user_turn(session_id, "escape"),
    );
    let outside = home.join("outside-worktree");
    write(&outside.join("keep.txt"), "must survive");
    let session_dir = home.join(format!(".grok/sessions/%2Fwork%2Fproject/{session_id}"));
    symlink(outside.join("keep.txt"), session_dir.join("escaped")).expect("create escaped symlink");
    let catalog = SessionCatalog::scan(home).expect("scan grok sessions");
    let session = catalog
        .resolve(Some("grok"), session_id)
        .expect("fixture session")
        .clone();
    catalog
        .delete_session(&session)
        .expect("unlink internal symlink");
    assert!(!session.path.exists());
    assert!(outside.join("keep.txt").exists());
}

fn write_indexed_grok_fixture(home: &std::path::Path) {
    write_grok_session(
        home,
        "%2Fwork%2Fgrok",
        "grok-id",
        r#"{"info":{"id":"grok-id","cwd":"/work/grok"},"session_summary":"","generated_title":"Fix Grok rendering","created_at":"2026-01-02T03:04:05Z","updated_at":"2026-01-02T03:05:05Z","num_messages":3,"current_model_id":"grok-4.6"}"#,
        concat!(
            r#"{"timestamp":1767323045,"method":"session/update","params":{"sessionId":"grok-id","update":{"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"Fix Grok rendering"},"_meta":{"modelId":"grok-4.6"}}}}"#,
            "\n",
            r#"{"timestamp":1767323046,"method":"session/update","params":{"sessionId":"grok-id","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Working."},"_meta":{"modelId":"grok-4.6"}}}}"#,
            "\n",
            r#"{"timestamp":1767323047,"method":"session/update","params":{"sessionId":"grok-id","update":{"sessionUpdate":"turn_completed","stop_reason":"end_turn","elapsed_ms":12,"usage":{"inputTokens":10,"outputTokens":20,"totalTokens":30,"costUsdTicks":99}}}}"#,
            "\n",
        ),
    );
}

fn assert_indexed_provider_sessions(catalog: &SessionCatalog) {
    assert_eq!(catalog.sessions().len(), 7);
    assert_session(
        catalog,
        &AgentKind::Codex,
        "codex-id",
        "Fix Codex rendering",
        120,
        None,
    );
    assert_session(
        catalog,
        &AgentKind::ClaudeCode,
        "claude-id",
        "Fix Claude rendering",
        100,
        None,
    );
    assert_session(
        catalog,
        &AgentKind::GeminiCli,
        "gemini-id",
        "Fix Gemini rendering",
        55,
        None,
    );
    assert_session(
        catalog,
        &AgentKind::OpenCode,
        "opencode-id",
        "Fix OpenCode rendering",
        65,
        Some(0.25),
    );
    assert_session(
        catalog,
        &AgentKind::Pi,
        "pi-id",
        "Fix Pi rendering",
        77,
        Some(0.5),
    );
    assert_session(
        catalog,
        &AgentKind::OhMyPi,
        "omp-id",
        "Fix OMP rendering",
        88,
        Some(0.75),
    );
    assert_indexed_grok_session(catalog);
}

fn assert_indexed_grok_session(catalog: &SessionCatalog) {
    assert_session(
        catalog,
        &AgentKind::Grok,
        "grok-id",
        "Fix Grok rendering",
        30,
        None,
    );
    let grok = catalog
        .sessions()
        .iter()
        .find(|session| session.kind == AgentKind::Grok)
        .expect("grok session");
    assert_eq!(
        grok.project.as_deref(),
        Some(std::path::Path::new("/work/grok"))
    );
    assert!(
        !grok
            .project
            .as_ref()
            .expect("decoded project")
            .to_string_lossy()
            .contains("%2F")
    );
}

fn write_grok_session(
    home: &std::path::Path,
    group: &str,
    id: &str,
    summary: &str,
    updates: impl AsRef<str>,
) -> PathBuf {
    write_grok_session_in(&home.join(".grok"), group, id, summary, updates)
}

fn write_grok_session_in(
    grok_home: &std::path::Path,
    group: &str,
    id: &str,
    summary: &str,
    updates: impl AsRef<str>,
) -> PathBuf {
    write_grok_session_at(
        &grok_home.join("sessions").join(group).join(id),
        summary,
        updates,
    )
}

fn write_grok_session_at(
    session_dir: &std::path::Path,
    summary: &str,
    updates: impl AsRef<str>,
) -> PathBuf {
    write(&session_dir.join("summary.json"), summary);
    let updates_path = session_dir.join("updates.jsonl");
    write(&updates_path, updates.as_ref());
    updates_path
}

fn grok_user_turn(id: &str, text: &str) -> String {
    format!(
        "{{\"timestamp\":1767323045,\"method\":\"session/update\",\"params\":{{\"sessionId\":\"{id}\",\"update\":{{\"sessionUpdate\":\"user_message_chunk\",\"content\":{{\"type\":\"text\",\"text\":{text:?}}},\"_meta\":{{\"modelId\":\"grok-4.6\"}}}}}}}}\n"
    )
}

fn assert_session(
    catalog: &SessionCatalog,
    kind: &AgentKind,
    id: &str,
    title: &str,
    tokens: u64,
    cost_usd: Option<f64>,
) {
    let session = catalog
        .sessions()
        .iter()
        .find(|session| &session.kind == kind)
        .expect("session kind");
    let detail = catalog.detail(session).expect("load session usage");
    assert_eq!(detail.session.id, id);
    assert_eq!(detail.session.title.as_deref(), Some(title));
    assert_eq!(detail.session.tokens, Some(tokens));
    assert_eq!(detail.session.cost_usd, cost_usd);
}

fn write(path: &std::path::Path, contents: &str) {
    fs::create_dir_all(path.parent().expect("parent")).expect("create fixture directory");
    fs::write(path, contents).expect("write fixture");
}

fn live_agent(
    kind: AgentKind,
    pid: u32,
    command: &[&str],
    cwd: &str,
    started_at: u64,
) -> LiveAgent {
    LiveAgent {
        kind,
        process: ProcessSnapshot {
            pid,
            parent_pid: Some(1),
            executable: PathBuf::from(command[0]),
            command: command.iter().map(ToString::to_string).collect(),
            cwd: Some(PathBuf::from(cwd)),
            started_at,
            run_time: 1,
            cpu_percent: 0.0,
            memory_bytes: 1,
            status: "running".to_owned(),
        },
    }
}
#[test]
#[allow(clippy::too_many_lines)]
fn loads_cursor_session_catalog_detail_and_deletes_records() {
    let temp = tempfile::tempdir().expect("temp dir");
    let home = temp.path();
    let global_dir = home.join("Library/Application Support/Cursor/User/globalStorage");
    fs::create_dir_all(&global_dir).expect("create global dir");
    let db_path = global_dir.join("state.vscdb");

    let connection = rusqlite::Connection::open(&db_path).expect("create test sqlite db");
    connection
        .execute_batch(
            "CREATE TABLE composerHeaders (
                    composerId TEXT PRIMARY KEY,
                    workspaceId TEXT,
                    createdAt INTEGER,
                    lastUpdatedAt INTEGER,
                    isArchived INTEGER,
                    isSubagent INTEGER,
                    recency INTEGER,
                    checkpointAt INTEGER,
                    value TEXT
                );
                CREATE TABLE cursorDiskKV (
                    key TEXT UNIQUE ON CONFLICT REPLACE,
                    value BLOB
                );",
        )
        .expect("create tables");

    let composer_id = "cursor-test-session-123";
    let header_json = serde_json::json!({
        "type": "head",
        "composerId": composer_id,
        "name": "Refactor Cursor Session Engine",
        "createdAt": 1_783_300_000_000_u64,
        "lastUpdatedAt": 1_783_300_000_000_u64,
        "workspaceIdentifier": {
            "uri": {
                "fsPath": "/work/cursor-project"
            }
        }
    })
    .to_string();

    connection
            .execute(
                "INSERT INTO composerHeaders (composerId, createdAt, lastUpdatedAt, value) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![composer_id, 1_783_300_000_000_i64, 1_783_300_000_000_i64, header_json],
            )
            .expect("insert header");

    let composer_data = serde_json::json!({
        "composerId": composer_id,
        "conversation": [
            {
                "type": 1,
                "text": "Hello Cursor",
                "createdAt": "2026-08-06T10:00:00Z"
            },
            {
                "type": 2,
                "text": "Hello! I can help you with Rust.",
                "createdAt": "2026-08-06T10:00:01Z",
                "model": "claude-3.7-sonnet"
            }
        ]
    })
    .to_string();

    connection
        .execute(
            "INSERT INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
            rusqlite::params![format!("composerData:{composer_id}"), composer_data],
        )
        .expect("insert composerData");

    drop(connection);

    let catalog = SessionCatalog::scan(home).expect("scan home");
    let cursor_sessions: Vec<_> = catalog
        .sessions()
        .iter()
        .filter(|s| s.kind == AgentKind::Cursor)
        .collect();

    assert_eq!(cursor_sessions.len(), 1);
    let session = cursor_sessions[0];
    assert_eq!(session.id, composer_id);
    assert_eq!(
        session.title.as_deref(),
        Some("Refactor Cursor Session Engine")
    );
    assert_eq!(
        session.project.as_deref(),
        Some(std::path::Path::new("/work/cursor-project"))
    );

    let detail = catalog.detail(session).expect("load detail");
    assert_eq!(detail.messages.len(), 2);
    assert_eq!(detail.messages[0].kind, SessionMessageKind::User);
    assert_eq!(detail.messages[0].content, "Hello Cursor");
    assert_eq!(detail.messages[1].kind, SessionMessageKind::Assistant);
    assert_eq!(
        detail.messages[1].content,
        "Hello! I can help you with Rust."
    );

    let summary = catalog
        .delete_session(session)
        .expect("delete cursor session");
    assert_eq!(summary.index_records, 2);
    assert!(db_path.exists());

    let rescan = SessionCatalog::scan(home).expect("rescan home");
    assert!(
        rescan
            .sessions()
            .iter()
            .all(|s| s.kind != AgentKind::Cursor || s.id != composer_id)
    );
}

#[test]
fn automatically_hides_messageless_empty_cursor_sessions() {
    let temp = tempfile::tempdir().expect("temp dir");
    let home = temp.path();
    let global_dir = home.join("Library/Application Support/Cursor/User/globalStorage");
    fs::create_dir_all(&global_dir).expect("create global dir");
    let db_path = global_dir.join("state.vscdb");

    let connection = rusqlite::Connection::open(&db_path).expect("create test sqlite db");
    connection
        .execute_batch(
            "CREATE TABLE composerHeaders (
                    composerId TEXT PRIMARY KEY,
                    workspaceId TEXT,
                    createdAt INTEGER,
                    lastUpdatedAt INTEGER,
                    isArchived INTEGER,
                    isSubagent INTEGER,
                    recency INTEGER,
                    checkpointAt INTEGER,
                    value TEXT
                );
                CREATE TABLE cursorDiskKV (
                    key TEXT UNIQUE ON CONFLICT REPLACE,
                    value BLOB
                );",
        )
        .expect("create tables");

    // 1. Populated session
    let populated_id = "cursor-populated-session";
    let populated_header = serde_json::json!({
        "type": "head",
        "composerId": populated_id,
        "createdAt": 1_783_300_000_000_u64,
        "workspaceIdentifier": { "uri": { "fsPath": "/work/project-a" } }
    })
    .to_string();
    let populated_data = serde_json::json!({
        "composerId": populated_id,
        "conversation": [{ "type": 1, "text": "Valid question" }]
    })
    .to_string();

    // 2. Messageless empty draft session
    let empty_id = "cursor-empty-draft-session";
    let empty_header = serde_json::json!({
        "type": "head",
        "composerId": empty_id,
        "createdAt": 1_783_300_000_000_u64,
        "workspaceIdentifier": { "uri": { "fsPath": "/work/project-a" } }
    })
    .to_string();

    connection
        .execute(
            "INSERT INTO composerHeaders (composerId, createdAt, value) VALUES (?1, ?2, ?3)",
            rusqlite::params![populated_id, 1_783_300_000_000_i64, populated_header],
        )
        .expect("insert populated header");
    connection
        .execute(
            "INSERT INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
            rusqlite::params![format!("composerData:{populated_id}"), populated_data],
        )
        .expect("insert populated data");

    connection
        .execute(
            "INSERT INTO composerHeaders (composerId, createdAt, value) VALUES (?1, ?2, ?3)",
            rusqlite::params![empty_id, 1_783_300_000_000_i64, empty_header],
        )
        .expect("insert empty header");

    drop(connection);

    let catalog = SessionCatalog::scan(home).expect("scan home");
    let sessions = catalog.sessions();

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].id, populated_id);
    assert_eq!(sessions[0].title.as_deref(), Some("Valid question"));

    let expanded =
        SessionCatalog::scan_provider(home, None, true, false).expect("scan home with drafts");
    let expanded_sessions = expanded.sessions();
    assert_eq!(expanded_sessions.len(), 2);
    assert!(
        expanded_sessions
            .iter()
            .any(|s| s.id == empty_id && s.title.is_none())
    );
}

#[test]
fn overlay_title_wins_over_native_and_clears_back() {
    let temp = tempdir().expect("temp home");
    let home = temp.path();
    write(
        &home.join(".local/share/opencode/storage/session/project/opencode.json"),
        r#"{"id":"opencode-id","directory":"/work/opencode","title":"Fix OpenCode rendering","time":{"created":1760000000000,"updated":1760000001000}}"#,
    );

    let catalog = SessionCatalog::scan(home).expect("scan sessions");
    let session = catalog
        .resolve(Some("opencode"), "opencode-id")
        .expect("session")
        .clone();
    assert_eq!(session.title.as_deref(), Some("Fix OpenCode rendering"));

    let renamed = catalog
        .set_title(&session, "  Custom   name  ")
        .expect("rename session");
    assert_eq!(renamed.as_deref(), Some("Custom name"));

    let catalog = SessionCatalog::scan(home).expect("rescan overlay");
    let session = catalog
        .resolve(Some("opencode"), "opencode-id")
        .expect("session")
        .clone();
    assert_eq!(session.title.as_deref(), Some("Custom name"));

    let restored = catalog.set_title(&session, "   ").expect("clear overlay");
    assert_eq!(restored.as_deref(), Some("Fix OpenCode rendering"));

    let catalog = SessionCatalog::scan(home).expect("rescan native");
    let session = catalog
        .resolve(Some("opencode"), "opencode-id")
        .expect("session");
    assert_eq!(session.title.as_deref(), Some("Fix OpenCode rendering"));
    assert!(
        !home.join(".config/mena/session-titles.toml").exists(),
        "empty overlay file should be removed"
    );
}

#[test]
fn overlay_is_removed_when_the_session_is_deleted() {
    let temp = tempdir().expect("temp home");
    let home = temp.path();
    let session_path =
        home.join(".local/share/opencode/storage/session/project/opencode-delete.json");
    write(
        &session_path,
        r#"{"id":"opencode-delete","directory":"/work/opencode","title":"Delete me","time":{"created":1760000000000,"updated":1760000001000}}"#,
    );
    write(
        &home.join(".local/share/opencode/storage/message/opencode-delete/message-delete.json"),
        r#"{"id":"message-delete","role":"assistant"}"#,
    );

    let catalog = SessionCatalog::scan(home).expect("scan sessions");
    let session = catalog
        .resolve(Some("opencode"), "opencode-delete")
        .expect("session")
        .clone();
    catalog
        .set_title(&session, "Keep this overlay")
        .expect("rename before delete");
    assert!(home.join(".config/mena/session-titles.toml").exists());

    catalog.delete_session(&session).expect("delete session");
    assert!(
        !home.join(".config/mena/session-titles.toml").exists(),
        "deleting a session must drop its overlay title"
    );
}

#[test]
fn set_title_rejects_sessions_outside_the_catalog() {
    let temp = tempdir().expect("temp home");
    let catalog = SessionCatalog::scan(temp.path()).expect("empty catalog");
    let stray = AgentSession {
        kind: AgentKind::OpenCode,
        id: "missing".to_owned(),
        title: None,
        project: None,
        path: temp.path().join("missing.json"),
        related_paths: BTreeSet::new(),
        started_at: None,
        updated_at: 0,
        tokens: None,
        cost_usd: None,
    };
    let error = catalog
        .set_title(&stray, "Nope")
        .expect_err("missing session");
    assert!(format!("{error:#}").contains("not in the current catalog"));
}

#[test]
fn set_title_rejects_overlong_titles() {
    let temp = tempdir().expect("temp home");
    let home = temp.path();
    write(
        &home.join(".local/share/opencode/storage/session/project/opencode.json"),
        r#"{"id":"opencode-id","directory":"/work/opencode","title":"Native","time":{"created":1,"updated":2}}"#,
    );
    let catalog = SessionCatalog::scan(home).expect("scan sessions");
    let session = catalog
        .resolve(Some("opencode"), "opencode-id")
        .expect("session");
    let error = catalog
        .set_title(session, &"a".repeat(121))
        .expect_err("overlong title");
    assert!(format!("{error:#}").contains("at most 120 characters"));
}
