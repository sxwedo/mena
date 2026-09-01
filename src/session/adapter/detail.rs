//! Native transcript decoders that normalize provider records into session messages.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Result, bail};
use serde_json::Value;

use super::super::{
    MAX_RECORD_BYTES, MetricError, ResponseMetrics, SessionMessage, SessionMessageKind,
    SessionMessageMetrics, TokenUsage, ToolMetrics, content_text_full, file_stem,
    files_with_extension, read_json_file, string_at, visit_bounded_lines,
};
use crate::AgentKind;

type Usage = (Option<u64>, Option<f64>);

#[derive(Default)]
pub(super) struct LoadedSession {
    pub(super) tokens: Option<u64>,
    pub(super) cost_usd: Option<f64>,
    pub(super) messages: Vec<SessionMessage>,
}

enum JsonlUsage {
    Codex {
        tokens: Option<u64>,
    },
    Claude {
        anonymous_tokens: u64,
        message_tokens: BTreeMap<String, u64>,
        has_usage: bool,
    },
    Pi(NumericUsage),
}

impl JsonlUsage {
    fn new(kind: &AgentKind) -> Self {
        match kind {
            AgentKind::Codex => Self::Codex { tokens: None },
            AgentKind::ClaudeCode => Self::Claude {
                anonymous_tokens: 0,
                message_tokens: BTreeMap::new(),
                has_usage: false,
            },
            AgentKind::Pi | AgentKind::OhMyPi => Self::Pi(NumericUsage::default()),
            _ => unreachable!("only JSONL providers have JSONL usage accumulators"),
        }
    }

    fn ingest(&mut self, record: &Value) {
        match self {
            Self::Codex { tokens } => {
                if record.pointer("/payload/type").and_then(Value::as_str) == Some("token_count")
                    && let Some(total) = record
                        .pointer("/payload/info/total_token_usage/total_tokens")
                        .and_then(Value::as_u64)
                {
                    *tokens = Some(total);
                }
            }
            Self::Claude {
                anonymous_tokens,
                message_tokens,
                has_usage,
            } => {
                if let Some(usage) = record.pointer("/message/usage")
                    && let Some(tokens) = token_usage_with_component_total(usage).total
                {
                    *has_usage = true;
                    if let Some(message_id) = string_at(record, "/message/id") {
                        message_tokens.insert(message_id, tokens);
                    } else {
                        *anonymous_tokens = anonymous_tokens.saturating_add(tokens);
                    }
                }
            }
            Self::Pi(usage) => usage.ingest_pi(record),
        }
    }

    fn finish(self, complete: bool) -> Usage {
        if !complete {
            return (None, None);
        }
        match self {
            Self::Codex { tokens } => (tokens, None),
            Self::Claude {
                anonymous_tokens,
                message_tokens,
                has_usage,
            } => {
                let tokens = message_tokens
                    .into_values()
                    .fold(anonymous_tokens, u64::saturating_add);
                (has_usage.then_some(tokens), None)
            }
            Self::Pi(usage) => usage.finish(),
        }
    }
}

#[derive(Default)]
struct NumericUsage {
    tokens: u64,
    has_tokens: bool,
    cost_usd: f64,
    has_cost: bool,
}

impl NumericUsage {
    fn ingest_pi(&mut self, record: &Value) {
        let Some(usage) = record.pointer("/message/usage") else {
            return;
        };
        if let Some(tokens) = usage.get("totalTokens").and_then(Value::as_u64) {
            self.has_tokens = true;
            self.tokens = self.tokens.saturating_add(tokens);
        }
        if let Some(cost) = usage.pointer("/cost/total").and_then(Value::as_f64) {
            self.has_cost = true;
            self.cost_usd += cost;
        }
    }

    fn ingest_opencode(&mut self, message: &Value) {
        if let Some(usage) = message.get("tokens") {
            self.has_tokens = true;
            for pointer in [
                "/input",
                "/output",
                "/reasoning",
                "/cache/read",
                "/cache/write",
            ] {
                self.tokens = self.tokens.saturating_add(
                    usage
                        .pointer(pointer)
                        .and_then(Value::as_u64)
                        .unwrap_or_default(),
                );
            }
        }
        if let Some(cost) = message.get("cost").and_then(Value::as_f64) {
            self.has_cost = true;
            self.cost_usd += cost;
        }
    }

    fn finish(self) -> Usage {
        (
            self.has_tokens.then_some(self.tokens),
            self.has_cost.then_some(self.cost_usd),
        )
    }
}

pub(super) fn codex_detail(path: &Path) -> Result<LoadedSession> {
    let mut usage = JsonlUsage::new(&AgentKind::Codex);
    let mut messages: Vec<SessionMessage> = Vec::new();
    let mut current_model = None;
    let mut pending_assistant: Option<usize> = None;
    let mut turn_last_assistant: Option<usize> = None;
    let mut tool_targets = BTreeMap::<String, usize>::new();
    let skipped = visit_bounded_lines(path, |line| {
        let Ok(record) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        usage.ingest(&record);
        let Some(payload) = record.get("payload") else {
            return;
        };
        if record.get("type").and_then(Value::as_str) == Some("turn_context") {
            current_model = model_id(payload);
            return;
        }
        let Some(payload_type) = payload.get("type").and_then(Value::as_str) else {
            return;
        };
        update_codex_metrics(
            payload_type,
            payload,
            &mut messages,
            &mut pending_assistant,
            &mut turn_last_assistant,
            &tool_targets,
        );
        let timestamp =
            string_at(&record, "/timestamp").or_else(|| string_at(&record, "/payload/timestamp"));
        let parsed = if payload_type == "message" {
            let role = payload
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("system");
            payload
                .get("content")
                .map(|content| {
                    let kind = SessionMessageKind::from_provider_role(role);
                    let model = (kind == SessionMessageKind::Assistant)
                        .then(|| model_id(payload).or_else(|| current_model.clone()))
                        .flatten();
                    messages_from_content(kind, timestamp, model, content)
                })
                .unwrap_or_default()
        } else {
            codex_event_message(payload_type, timestamp, payload)
        };
        let start = messages.len();
        messages.extend(parsed);
        if matches!(payload_type, "function_call" | "custom_tool_call")
            && let Some(call_id) = string_at(payload, "/call_id")
            && let Some(relative_index) = messages[start..].iter().position(|message| {
                matches!(
                    message.kind,
                    SessionMessageKind::ToolCall | SessionMessageKind::Skill
                )
            })
        {
            tool_targets.insert(call_id, start + relative_index);
        }
        if messages[start..]
            .iter()
            .any(|message| message.kind == SessionMessageKind::User)
        {
            pending_assistant = None;
            turn_last_assistant = None;
        }
        if let Some(relative_index) = messages[start..]
            .iter()
            .rposition(|message| message.kind == SessionMessageKind::Assistant)
        {
            let index = start + relative_index;
            pending_assistant = Some(index);
            turn_last_assistant = Some(index);
        }
    })?;
    ensure_complete(path, skipped)?;
    let (tokens, cost_usd) = usage.finish(true);
    Ok(LoadedSession {
        tokens,
        cost_usd,
        messages,
    })
}

fn update_codex_metrics(
    payload_type: &str,
    payload: &Value,
    messages: &mut [SessionMessage],
    pending_assistant: &mut Option<usize>,
    turn_last_assistant: &mut Option<usize>,
    tool_targets: &BTreeMap<String, usize>,
) {
    if payload_type == "task_started" {
        *pending_assistant = None;
        *turn_last_assistant = None;
    } else if payload_type == "token_count" {
        if let Some(index) = pending_assistant.take()
            && let Some(usage) = payload.pointer("/info/last_token_usage")
        {
            messages[index].metrics.response_mut().tokens = token_usage(usage);
        }
    } else if matches!(payload_type, "task_complete" | "turn_aborted")
        && let Some(index) = *turn_last_assistant
    {
        let response = messages[index].metrics.response_mut();
        response.duration_ms = payload.get("duration_ms").and_then(number_as_milliseconds);
        response.time_to_first_token_ms = payload
            .get("time_to_first_token_ms")
            .and_then(number_as_milliseconds);
        response.finish_reason = string_at(payload, "/reason")
            .or_else(|| string_at(payload, "/status"))
            .or_else(|| Some(payload_type.to_owned()));
    } else if matches!(payload_type, "mcp_tool_call_end" | "patch_apply_end")
        && let Some(call_id) = string_at(payload, "/call_id")
        && let Some(index) = tool_targets.get(&call_id).copied()
    {
        let tool = messages[index].metrics.tool_mut();
        tool.duration_ms = codex_tool_duration_ms(payload);
        tool.status = codex_tool_status(payload);
        tool.exit_code = payload.get("exit_code").and_then(Value::as_i64);
        tool.error = codex_tool_error(payload);
    }
}

fn codex_tool_duration_ms(payload: &Value) -> Option<u64> {
    payload
        .get("duration_ms")
        .and_then(number_as_milliseconds)
        .or_else(|| {
            let duration = payload.get("duration")?;
            let seconds = duration.get("secs")?.as_u64()?;
            let nanos = duration
                .get("nanos")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            Some(
                seconds
                    .saturating_mul(1_000)
                    .saturating_add(nanos / 1_000_000),
            )
        })
}

fn codex_tool_status(payload: &Value) -> Option<String> {
    string_at(payload, "/status")
        .or_else(|| {
            payload
                .get("success")
                .and_then(Value::as_bool)
                .map(|success| {
                    if success {
                        "completed".to_owned()
                    } else {
                        "error".to_owned()
                    }
                })
        })
        .or_else(|| {
            let result = payload.get("result")?;
            if result.get("Ok").is_some() {
                Some("completed".to_owned())
            } else if result.get("Err").is_some() {
                Some("error".to_owned())
            } else {
                None
            }
        })
}

fn codex_tool_error(payload: &Value) -> Option<MetricError> {
    metric_error(payload)
        .or_else(|| {
            string_at(payload, "/stderr")
                .filter(|message| !message.trim().is_empty())
                .map(|message| MetricError {
                    code: None,
                    message,
                })
        })
        .or_else(|| {
            let error = payload.pointer("/result/Err")?;
            Some(MetricError {
                code: None,
                message: content_text_full(error)?,
            })
        })
}

fn codex_event_message(
    payload_type: &str,
    timestamp: Option<String>,
    payload: &Value,
) -> Vec<SessionMessage> {
    let kind = if payload_type == "error" {
        SessionMessageKind::Error
    } else if payload_type.contains("call_output") || payload_type.ends_with("_result") {
        SessionMessageKind::ToolResult
    } else if payload_type.ends_with("_call") {
        tool_call_kind(payload)
    } else {
        SessionMessageKind::System
    };
    content_text_full(payload)
        .map(|content| {
            vec![SessionMessage {
                kind,
                timestamp,
                model: None,
                metrics: SessionMessageMetrics::default(),
                content,
            }]
        })
        .unwrap_or_default()
}

pub(super) fn nested_jsonl_detail(path: &Path, kind: &AgentKind) -> Result<LoadedSession> {
    let mut usage = JsonlUsage::new(kind);
    let mut messages: Vec<SessionMessage> = Vec::new();
    let mut metric_targets = BTreeMap::new();
    let mut tool_targets: BTreeMap<String, (usize, Option<u64>)> = BTreeMap::new();
    let skipped = visit_bounded_lines(path, |line| {
        let Ok(record) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        usage.ingest(&record);
        let Some(role) = record.pointer("/message/role").and_then(Value::as_str) else {
            return;
        };
        let Some(content) = record.pointer("/message/content") else {
            return;
        };
        let timestamp =
            string_at(&record, "/timestamp").or_else(|| string_at(&record, "/message/timestamp"));
        let model = record.pointer("/message").and_then(model_id);
        let parent_kind = SessionMessageKind::from_provider_role(role);
        let mut parsed =
            parsed_messages_from_content(parent_kind, timestamp.clone(), model.clone(), content);
        if let Some(tool_id) = string_at(&record, "/message/toolCallId")
            .or_else(|| string_at(&record, "/message/tool_use_id"))
        {
            for parsed in &mut parsed {
                if parsed.message.kind == SessionMessageKind::ToolResult
                    && parsed.tool_call_id.is_none()
                {
                    parsed.tool_call_id = Some(tool_id.clone());
                }
            }
        }
        let start = messages.len();
        attach_nested_response_metrics(
            &record,
            parent_kind,
            model,
            start,
            &mut parsed,
            &mut metric_targets,
            &mut messages,
        );
        let timestamp_ms = timestamp.as_deref().and_then(timestamp_millis);
        let tool_events = nested_tool_events(&parsed, start);
        messages.extend(parsed.into_iter().map(|parsed| parsed.message));
        correlate_nested_tools(
            &record,
            timestamp_ms,
            tool_events,
            &mut messages,
            &mut tool_targets,
        );
    })?;
    ensure_complete(path, skipped)?;
    let (tokens, cost_usd) = usage.finish(true);
    Ok(LoadedSession {
        tokens,
        cost_usd,
        messages,
    })
}

#[derive(Debug)]
struct NestedToolEvent {
    kind: SessionMessageKind,
    id: String,
    target: usize,
    status: Option<String>,
    error: Option<MetricError>,
}

fn attach_nested_response_metrics(
    record: &Value,
    parent_kind: SessionMessageKind,
    model: Option<String>,
    start: usize,
    parsed: &mut [ParsedMessage],
    metric_targets: &mut BTreeMap<String, usize>,
    messages: &mut [SessionMessage],
) {
    if parent_kind != SessionMessageKind::Assistant {
        return;
    }
    let response_target = parsed
        .iter()
        .rposition(|parsed| parsed.message.kind == SessionMessageKind::Assistant)
        .or_else(|| {
            parsed.iter().rposition(|parsed| {
                matches!(
                    parsed.message.kind,
                    SessionMessageKind::ToolCall | SessionMessageKind::Skill
                )
            })
        });
    let Some(index) = response_target else {
        return;
    };
    parsed[index].message.model = model;
    let target = start + index;
    if let Some(message_id) = string_at(record, "/message/id")
        && let Some(previous) = metric_targets.insert(message_id, target)
    {
        messages[previous].metrics = SessionMessageMetrics::default();
    }
    let native_message = record.pointer("/message").unwrap_or(record);
    parsed[index].message.metrics.response = Some(ResponseMetrics {
        duration_ms: record
            .pointer("/message/duration")
            .and_then(number_as_milliseconds),
        time_to_first_token_ms: record
            .pointer("/message/ttft")
            .and_then(number_as_milliseconds),
        cost_usd: record
            .pointer("/message/usage/cost/total")
            .and_then(Value::as_f64),
        finish_reason: string_at(record, "/message/stopReason")
            .or_else(|| string_at(record, "/message/stop_reason")),
        retry_count: response_retry_count(native_message),
        error: metric_error(native_message),
        tokens: record
            .pointer("/message/usage")
            .map_or_else(TokenUsage::default, token_usage_with_component_total),
    });
}

fn nested_tool_events(parsed: &[ParsedMessage], start: usize) -> Vec<NestedToolEvent> {
    parsed
        .iter()
        .enumerate()
        .filter_map(|(index, parsed)| {
            Some(NestedToolEvent {
                kind: parsed.message.kind,
                id: parsed.tool_call_id.clone()?,
                target: start + index,
                status: parsed.tool_status.clone(),
                error: parsed.tool_error.clone(),
            })
        })
        .collect()
}

fn correlate_nested_tools(
    record: &Value,
    timestamp_ms: Option<u64>,
    events: Vec<NestedToolEvent>,
    messages: &mut [SessionMessage],
    tool_targets: &mut BTreeMap<String, (usize, Option<u64>)>,
) {
    for event in events {
        if matches!(
            event.kind,
            SessionMessageKind::ToolCall | SessionMessageKind::Skill
        ) {
            tool_targets.insert(event.id, (event.target, timestamp_ms));
            continue;
        }
        let Some((call_target, started_at)) = tool_targets.get(&event.id).copied() else {
            continue;
        };
        let is_error = record.pointer("/message/isError").and_then(Value::as_bool);
        let tool = messages[call_target].metrics.tool_mut();
        tool.status = event.status.or_else(|| {
            is_error.map(|is_error| {
                if is_error {
                    "error".to_owned()
                } else {
                    "completed".to_owned()
                }
            })
        });
        tool.duration_ms = record
            .pointer("/message/details/wallTimeMs")
            .and_then(number_as_milliseconds)
            .or_else(|| {
                timestamp_ms
                    .zip(started_at)
                    .map(|(ended, started)| ended.saturating_sub(started))
            });
        tool.exit_code = record
            .pointer("/message/details/exitCode")
            .and_then(Value::as_i64);
        tool.error = event
            .error
            .or_else(|| metric_error(record.pointer("/message").unwrap_or(record)));
    }
}

pub(super) fn gemini_detail(path: &Path) -> Result<LoadedSession> {
    let Some(session) = read_json_file(path)? else {
        return Ok(LoadedSession::default());
    };
    let (tokens, cost_usd) = gemini_usage(&session);
    Ok(LoadedSession {
        tokens,
        cost_usd,
        messages: gemini_messages(&session),
    })
}

fn gemini_usage(session: &Value) -> Usage {
    let tokens = session
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|messages| {
            let mut total = 0_u64;
            let mut found = false;
            for tokens in messages
                .iter()
                .filter_map(|message| message.pointer("/tokens/total").and_then(Value::as_u64))
            {
                found = true;
                total = total.saturating_add(tokens);
            }
            found.then_some(total)
        });
    (tokens, None)
}

fn gemini_messages(session: &Value) -> Vec<SessionMessage> {
    let Some(values) = session.get("messages").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut messages = Vec::new();
    for message in values {
        let role = message
            .get("type")
            .or_else(|| message.get("role"))
            .and_then(Value::as_str);
        let content = message.get("content");
        let Some((role, content)) = role.zip(content) else {
            continue;
        };
        let mut parsed = messages_from_content(
            SessionMessageKind::from_provider_role(role),
            string_at(message, "/timestamp"),
            model_id(message),
            content,
        );
        if let Some(tool_calls) = message.get("toolCalls").and_then(Value::as_array) {
            for call in tool_calls {
                let mut call_content = call.clone();
                if let Some(object) = call_content.as_object_mut() {
                    object.remove("result");
                }
                if let Some(content) = content_text_full(&call_content) {
                    parsed.push(SessionMessage {
                        kind: tool_call_kind(call),
                        timestamp: string_at(call, "/timestamp")
                            .or_else(|| string_at(message, "/timestamp")),
                        model: None,
                        metrics: SessionMessageMetrics {
                            response: None,
                            tool: tool_metrics(call),
                        },
                        content,
                    });
                }
                if let Some(result) = call.get("result")
                    && let Some(content) = content_text_full(result)
                {
                    parsed.push(SessionMessage {
                        kind: SessionMessageKind::ToolResult,
                        timestamp: string_at(call, "/timestamp")
                            .or_else(|| string_at(message, "/timestamp")),
                        model: None,
                        metrics: SessionMessageMetrics::default(),
                        content,
                    });
                }
            }
        }
        let response_target = parsed
            .iter()
            .rposition(|message| message.kind == SessionMessageKind::Assistant)
            .or_else(|| {
                parsed.iter().rposition(|message| {
                    matches!(
                        message.kind,
                        SessionMessageKind::ToolCall | SessionMessageKind::Skill
                    )
                })
            });
        if SessionMessageKind::from_provider_role(role) == SessionMessageKind::Assistant
            && let Some(index) = response_target
        {
            parsed[index].model = model_id(message);
            parsed[index].metrics.response = Some(ResponseMetrics {
                duration_ms: message
                    .get("durationMs")
                    .and_then(number_as_milliseconds)
                    .or_else(|| {
                        message
                            .pointer("/metrics/durationMs")
                            .and_then(number_as_milliseconds)
                    }),
                finish_reason: string_at(message, "/finishReason")
                    .or_else(|| string_at(message, "/stopReason")),
                retry_count: response_retry_count(message),
                error: metric_error(message),
                tokens: message
                    .get("tokens")
                    .or_else(|| message.get("usage"))
                    .map_or_else(TokenUsage::default, token_usage_with_component_total),
                ..ResponseMetrics::default()
            });
        }
        messages.extend(parsed);
    }
    messages
}

pub(super) fn opencode_detail(home: &Path, session_id: &str) -> Result<LoadedSession> {
    let storage = home.join(".local/share/opencode/storage");
    let mut usage = NumericUsage::default();
    let mut messages = Vec::new();
    visit_opencode_messages(home, session_id, |message_path, message| {
        usage.ingest_opencode(message);
        let Some(role) = message.get("role").and_then(Value::as_str) else {
            return Ok(());
        };
        let Some(message_id) = string_at(message, "/id").or_else(|| file_stem(message_path)) else {
            return Ok(());
        };
        let created = message.pointer("/time/created").and_then(Value::as_u64);
        let timestamp = message
            .pointer("/time/created")
            .map(Value::to_string)
            .or_else(|| string_at(message, "/timestamp"));
        let parent_kind = SessionMessageKind::from_provider_role(role);
        let model = model_id(message);
        let mut part_paths = files_with_extension(&storage.join("part").join(&message_id), "json")?;
        part_paths.sort();
        let mut parsed = Vec::new();
        let mut first_part_started = None;
        for part_path in part_paths {
            let Some(part) = read_json_file(&part_path)? else {
                continue;
            };
            if let Some(started) = part.pointer("/time/start").and_then(Value::as_u64) {
                first_part_started =
                    Some(first_part_started.map_or(started, |value: u64| value.min(started)));
            }
            parsed.extend(opencode_part_messages(
                parent_kind,
                timestamp.clone(),
                model.clone(),
                &part,
            ));
        }
        if parsed.is_empty()
            && let Some(content) = message.get("content")
        {
            parsed.extend(messages_from_content(
                parent_kind,
                timestamp,
                model,
                content,
            ));
        }
        if let Some(index) = parsed
            .iter()
            .rposition(|message| message.kind == SessionMessageKind::Assistant)
        {
            parsed[index].metrics.response = Some(ResponseMetrics {
                duration_ms: message
                    .pointer("/time/completed")
                    .and_then(Value::as_u64)
                    .zip(created)
                    .map(|(completed, created)| completed.saturating_sub(created)),
                time_to_first_token_ms: first_part_started
                    .zip(created)
                    .map(|(started, created)| started.saturating_sub(created)),
                cost_usd: message.get("cost").and_then(Value::as_f64),
                finish_reason: string_at(message, "/finish"),
                retry_count: response_retry_count(message),
                error: metric_error(message),
                tokens: message
                    .get("tokens")
                    .map_or_else(TokenUsage::default, token_usage_with_component_total),
            });
        }
        for (part_index, message) in parsed.into_iter().enumerate() {
            messages.push((
                created.unwrap_or_default(),
                message_path.to_path_buf(),
                part_index,
                message,
            ));
        }
        Ok(())
    })?;
    messages.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    let (tokens, cost_usd) = usage.finish();
    Ok(LoadedSession {
        tokens,
        cost_usd,
        messages: messages
            .into_iter()
            .map(|(_, _, _, message)| message)
            .collect(),
    })
}

fn visit_opencode_messages(
    home: &Path,
    session_id: &str,
    mut visitor: impl FnMut(&Path, &Value) -> Result<()>,
) -> Result<()> {
    let root = home
        .join(".local/share/opencode/storage/message")
        .join(session_id);
    for path in files_with_extension(&root, "json")? {
        if let Some(message) = read_json_file(&path)? {
            visitor(&path, &message)?;
        }
    }
    Ok(())
}

#[derive(Debug)]
struct ParsedMessage {
    message: SessionMessage,
    tool_call_id: Option<String>,
    tool_status: Option<String>,
    tool_error: Option<MetricError>,
}

fn messages_from_content(
    parent_kind: SessionMessageKind,
    timestamp: Option<String>,
    model: Option<String>,
    content: &Value,
) -> Vec<SessionMessage> {
    parsed_messages_from_content(parent_kind, timestamp, model, content)
        .into_iter()
        .map(|parsed| parsed.message)
        .collect()
}

fn parsed_messages_from_content(
    parent_kind: SessionMessageKind,
    timestamp: Option<String>,
    model: Option<String>,
    content: &Value,
) -> Vec<ParsedMessage> {
    if let Value::Array(parts) = content {
        return parts
            .iter()
            .flat_map(|part| {
                parsed_messages_from_content(parent_kind, timestamp.clone(), model.clone(), part)
            })
            .collect();
    }

    let kind = content
        .get("type")
        .and_then(Value::as_str)
        .map_or(parent_kind, |value| {
            match value.to_ascii_lowercase().as_str() {
                "text" | "input_text" | "output_text" => parent_kind,
                "tool_use" | "tool_call" | "toolcall" | "function_call" => tool_call_kind(content),
                "tool_result" | "toolresult" | "function_call_output" => {
                    SessionMessageKind::ToolResult
                }
                "error" => SessionMessageKind::Error,
                "thinking" | "reasoning" | "system" | "meta" | "developer_message" => {
                    SessionMessageKind::System
                }
                _ => SessionMessageKind::System,
            }
        });
    let tool_call_id = tool_call_id(kind, content);
    let tool_is_error = (kind == SessionMessageKind::ToolResult)
        .then(|| content.get("is_error").and_then(Value::as_bool))
        .flatten();
    content_text_full(content)
        .map(|rendered_content| {
            vec![ParsedMessage {
                tool_call_id,
                tool_status: tool_is_error.map(|is_error| {
                    if is_error {
                        "error".to_owned()
                    } else {
                        "completed".to_owned()
                    }
                }),
                tool_error: tool_is_error
                    .filter(|is_error| *is_error)
                    .map(|_| MetricError {
                        code: None,
                        message: content
                            .get("content")
                            .and_then(Value::as_str)
                            .map_or_else(|| rendered_content.clone(), str::to_owned),
                    }),
                message: SessionMessage {
                    kind,
                    timestamp,
                    model: (kind == SessionMessageKind::Assistant)
                        .then_some(model)
                        .flatten(),
                    metrics: SessionMessageMetrics::default(),
                    content: rendered_content,
                },
            }]
        })
        .unwrap_or_default()
}

fn tool_call_id(kind: SessionMessageKind, value: &Value) -> Option<String> {
    let pointers: &[&str] = match kind {
        SessionMessageKind::ToolCall | SessionMessageKind::Skill => {
            &["/id", "/call_id", "/callId", "/toolCallId"]
        }
        SessionMessageKind::ToolResult => &["/tool_use_id", "/toolCallId", "/call_id", "/callId"],
        _ => return None,
    };
    pointers
        .iter()
        .find_map(|pointer| string_at(value, pointer))
}

fn timestamp_millis(value: &str) -> Option<u64> {
    value.parse().ok().or_else(|| {
        chrono::DateTime::parse_from_rfc3339(value)
            .ok()
            .and_then(|timestamp| timestamp.timestamp_millis().try_into().ok())
    })
}

fn opencode_part_messages(
    parent_kind: SessionMessageKind,
    timestamp: Option<String>,
    model: Option<String>,
    part: &Value,
) -> Vec<SessionMessage> {
    if part.get("type").and_then(Value::as_str) != Some("tool") {
        return messages_from_content(parent_kind, timestamp, model, part);
    }

    let mut messages = Vec::new();
    for (pointer, kind) in [
        ("/state/input", tool_call_kind(part)),
        ("/state/output", SessionMessageKind::ToolResult),
        ("/state/error", SessionMessageKind::Error),
    ] {
        let Some(value) = part.pointer(pointer) else {
            continue;
        };
        let mut object = serde_json::Map::new();
        if let Some(tool) = part.get("tool").or_else(|| part.get("name")) {
            object.insert("tool".to_owned(), tool.clone());
        }
        object.insert(
            pointer.rsplit('/').next().unwrap_or("value").to_owned(),
            value.clone(),
        );
        if let Some(content) = content_text_full(&Value::Object(object)) {
            messages.push(SessionMessage {
                kind,
                timestamp: timestamp.clone(),
                model: None,
                metrics: SessionMessageMetrics {
                    tool: (pointer == "/state/input")
                        .then(|| tool_metrics(part))
                        .flatten(),
                    ..SessionMessageMetrics::default()
                },
                content,
            });
        }
    }
    if messages.is_empty()
        && let Some(content) = content_text_full(part)
    {
        messages.push(SessionMessage {
            kind: tool_call_kind(part),
            timestamp,
            model: None,
            metrics: SessionMessageMetrics::default(),
            content,
        });
    }
    messages
}

fn tool_call_kind(value: &Value) -> SessionMessageKind {
    let name = value
        .get("name")
        .or_else(|| value.get("tool"))
        .or_else(|| value.pointer("/function/name"))
        .and_then(Value::as_str);
    if name.is_some_and(|name| name.to_ascii_lowercase().contains("skill")) {
        SessionMessageKind::Skill
    } else {
        SessionMessageKind::ToolCall
    }
}

fn model_id(value: &Value) -> Option<String> {
    [
        "/model",
        "/modelId",
        "/modelID",
        "/model/id",
        "/model/modelId",
        "/model/modelID",
    ]
    .into_iter()
    .find_map(|pointer| string_at(value, pointer))
}

fn response_retry_count(value: &Value) -> Option<u64> {
    u64_at(value, &["/retryCount", "/retry_count", "/retries"])
}

fn metric_error(value: &Value) -> Option<MetricError> {
    let message = ["/errorMessage", "/error/message", "/result/error/message"]
        .into_iter()
        .find_map(|pointer| string_at(value, pointer))
        .or_else(|| {
            value
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .or_else(|| {
            value
                .pointer("/result/error")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })?;
    let code = [
        "/errorId",
        "/error/code",
        "/error/type",
        "/error/name",
        "/result/error/code",
    ]
    .into_iter()
    .find_map(|pointer| string_at(value, pointer));
    Some(MetricError { code, message })
}

fn tool_metrics(value: &Value) -> Option<ToolMetrics> {
    let state = value.get("state").unwrap_or(value);
    let duration_ms = state
        .pointer("/time/end")
        .and_then(Value::as_u64)
        .zip(state.pointer("/time/start").and_then(Value::as_u64))
        .map(|(end, start)| end.saturating_sub(start))
        .or_else(|| state.get("durationMs").and_then(number_as_milliseconds))
        .or_else(|| state.get("wallTimeMs").and_then(number_as_milliseconds));
    let metrics = ToolMetrics {
        status: string_at(state, "/status"),
        duration_ms,
        exit_code: ["/exitCode", "/exit_code", "/metadata/exitCode"]
            .into_iter()
            .find_map(|pointer| state.pointer(pointer).and_then(Value::as_i64)),
        error: metric_error(state),
    };
    (metrics != ToolMetrics::default()).then_some(metrics)
}

fn token_usage(usage: &Value) -> TokenUsage {
    let five_minute_cache_write = u64_at(usage, &["/cache_creation/ephemeral_5m_input_tokens"]);
    let one_hour_cache_write = u64_at(usage, &["/cache_creation/ephemeral_1h_input_tokens"]);
    let cache_write = u64_at(
        usage,
        &[
            "/cache_creation_input_tokens",
            "/cache/write",
            "/cacheWrite",
        ],
    )
    .or_else(|| optional_sum([five_minute_cache_write, one_hour_cache_write]));
    TokenUsage {
        total: u64_at(usage, &["/total_tokens", "/totalTokens", "/total"]),
        input: u64_at(usage, &["/input_tokens", "/input"]),
        output: u64_at(usage, &["/output_tokens", "/output"]),
        cache_read: u64_at(
            usage,
            &[
                "/cached_input_tokens",
                "/cache_read_input_tokens",
                "/cache/read",
                "/cacheRead",
                "/cached",
            ],
        ),
        cache_write,
        cache_write_5m: five_minute_cache_write,
        cache_write_1h: one_hour_cache_write,
        reasoning: u64_at(
            usage,
            &[
                "/reasoning_output_tokens",
                "/reasoning",
                "/reasoningTokens",
                "/thoughts",
            ],
        ),
        tool: u64_at(usage, &["/tool", "/toolUsePromptTokenCount"]),
    }
}

fn optional_sum<const N: usize>(values: [Option<u64>; N]) -> Option<u64> {
    values
        .iter()
        .any(Option::is_some)
        .then(|| values.into_iter().flatten().fold(0, u64::saturating_add))
}

fn token_usage_with_component_total(usage: &Value) -> TokenUsage {
    let mut usage = token_usage(usage);
    if usage.total.is_none() {
        let components = [
            usage.input,
            usage.output,
            usage.cache_read,
            usage.cache_write,
            usage.reasoning,
            usage.tool,
        ];
        let found = components.iter().any(Option::is_some);
        usage.total = found.then(|| {
            components
                .into_iter()
                .flatten()
                .fold(0_u64, u64::saturating_add)
        });
    }
    usage
}

fn u64_at(value: &Value, pointers: &[&str]) -> Option<u64> {
    pointers
        .iter()
        .find_map(|pointer| value.pointer(pointer).and_then(Value::as_u64))
}

fn number_as_milliseconds(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_f64()
            .filter(|value| value.is_finite() && *value >= 0.0)
            .and_then(|value| format!("{value:.0}").parse().ok())
    })
}

struct GrokDetailState {
    messages: Vec<SessionMessage>,
    pending_chunk: Option<(SessionMessageKind, usize)>,
    tool_targets: BTreeMap<String, usize>,
    last_assistant: Option<usize>,
    tokens: u64,
    has_tokens: bool,
}

pub(super) fn grok_detail(path: &Path) -> Result<LoadedSession> {
    let mut state = GrokDetailState {
        messages: Vec::new(),
        pending_chunk: None,
        tool_targets: BTreeMap::new(),
        last_assistant: None,
        tokens: 0,
        has_tokens: false,
    };
    let skipped = visit_bounded_lines(path, |line| {
        let Ok(record) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        grok_apply_record(&mut state, &record);
    })?;
    ensure_complete(path, skipped)?;
    Ok(LoadedSession {
        tokens: state.has_tokens.then_some(state.tokens),
        cost_usd: None,
        messages: state.messages,
    })
}

fn grok_apply_record(state: &mut GrokDetailState, record: &Value) {
    let Some(update) = record.pointer("/params/update") else {
        return;
    };
    let Some(kind) = update.get("sessionUpdate").and_then(Value::as_str) else {
        return;
    };
    let timestamp = grok_event_timestamp(record);
    let model = string_at(update, "/_meta/modelId");
    match kind {
        "user_message_chunk" => grok_append_chunk(
            &mut state.messages,
            &mut state.pending_chunk,
            SessionMessageKind::User,
            timestamp,
            None,
            grok_chunk_text(update),
        ),
        "agent_message_chunk" => {
            grok_append_chunk(
                &mut state.messages,
                &mut state.pending_chunk,
                SessionMessageKind::Assistant,
                timestamp,
                model,
                grok_chunk_text(update),
            );
            if let Some((_, index)) = state.pending_chunk {
                state.last_assistant = Some(index);
            }
        }
        "agent_thought_chunk" => grok_append_chunk(
            &mut state.messages,
            &mut state.pending_chunk,
            SessionMessageKind::System,
            timestamp,
            model,
            grok_chunk_text(update),
        ),
        "tool_call" => grok_apply_tool_call(state, update, timestamp),
        "tool_call_update" => grok_apply_tool_update(state, update, timestamp),
        "turn_completed" => grok_apply_turn_completed(state, update),
        _ => state.pending_chunk = None,
    }
}

fn grok_apply_tool_call(state: &mut GrokDetailState, update: &Value, timestamp: Option<String>) {
    state.pending_chunk = None;
    let tool_id = string_at(update, "/toolCallId");
    let title = string_at(update, "/title");
    let mut object = serde_json::Map::new();
    if let Some(title) = title.clone() {
        object.insert("tool".to_owned(), Value::String(title));
    }
    if let Some(input) = update.get("rawInput") {
        object.insert("input".to_owned(), input.clone());
    }
    let Some(content) = content_text_full(&Value::Object(object)).or_else(|| title.clone()) else {
        return;
    };
    let index = state.messages.len();
    state.messages.push(SessionMessage {
        kind: tool_call_kind(update),
        timestamp,
        model: None,
        metrics: SessionMessageMetrics::default(),
        content,
    });
    if let Some(tool_id) = tool_id {
        state.tool_targets.insert(tool_id, index);
    }
}

fn grok_apply_tool_update(state: &mut GrokDetailState, update: &Value, timestamp: Option<String>) {
    state.pending_chunk = None;
    if let Some(tool_id) = string_at(update, "/toolCallId")
        && let Some(index) = state.tool_targets.get(&tool_id).copied()
    {
        let tool = state.messages[index].metrics.tool_mut();
        if let Some(status) = string_at(update, "/status") {
            tool.status = Some(status);
        }
        if let Some(error) = metric_error(update) {
            tool.error = Some(error);
        }
    }
    if let Some(content) = update.get("content").and_then(content_text_full) {
        state.messages.push(SessionMessage {
            kind: SessionMessageKind::ToolResult,
            timestamp,
            model: None,
            metrics: SessionMessageMetrics::default(),
            content,
        });
    }
}

fn grok_apply_turn_completed(state: &mut GrokDetailState, update: &Value) {
    state.pending_chunk = None;
    let Some(usage) = update.get("usage") else {
        return;
    };
    let parsed = grok_token_usage(usage);
    if let Some(total) = parsed
        .total
        .or_else(|| optional_sum([parsed.input, parsed.output, parsed.reasoning]))
    {
        state.has_tokens = true;
        state.tokens = state.tokens.saturating_add(total);
    }
    let Some(index) = state.last_assistant else {
        return;
    };
    let response = state.messages[index].metrics.response_mut();
    response.tokens = parsed;
    response.duration_ms = update.get("elapsed_ms").and_then(number_as_milliseconds);
    response.finish_reason = string_at(update, "/stop_reason");
    if let Some(model) = usage
        .get("modelUsage")
        .and_then(Value::as_object)
        .and_then(|models| models.keys().next().cloned())
    {
        state.messages[index].model.get_or_insert(model);
    }
}

fn grok_append_chunk(
    messages: &mut Vec<SessionMessage>,
    pending: &mut Option<(SessionMessageKind, usize)>,
    kind: SessionMessageKind,
    timestamp: Option<String>,
    model: Option<String>,
    text: Option<String>,
) {
    let Some(text) = text.filter(|value| !value.is_empty()) else {
        return;
    };
    if let Some((pending_kind, index)) = *pending
        && pending_kind == kind
    {
        messages[index].content.push_str(&text);
        return;
    }
    let index = messages.len();
    messages.push(SessionMessage {
        kind,
        timestamp,
        model,
        metrics: SessionMessageMetrics::default(),
        content: text,
    });
    *pending = Some((kind, index));
}

fn grok_chunk_text(update: &Value) -> Option<String> {
    update.get("content").and_then(content_text_full)
}

fn grok_event_timestamp(record: &Value) -> Option<String> {
    record
        .get("timestamp")
        .and_then(Value::as_i64)
        .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
        .map(|timestamp| timestamp.to_rfc3339())
        .or_else(|| string_at(record, "/timestamp"))
}

fn grok_token_usage(usage: &Value) -> TokenUsage {
    TokenUsage {
        total: usage.get("totalTokens").and_then(Value::as_u64),
        input: usage.get("inputTokens").and_then(Value::as_u64),
        output: usage.get("outputTokens").and_then(Value::as_u64),
        cache_read: usage.get("cachedReadTokens").and_then(Value::as_u64),
        cache_write: usage.get("cacheCreationTokens").and_then(Value::as_u64),
        reasoning: usage.get("reasoningTokens").and_then(Value::as_u64),
        ..TokenUsage::default()
    }
}

fn ensure_complete(path: &Path, skipped: bool) -> Result<()> {
    if skipped {
        bail!(
            "cannot show the complete transcript because {} contains a record larger than {MAX_RECORD_BYTES} bytes",
            path.display()
        );
    }
    Ok(())
}
#[allow(
    clippy::unnecessary_wraps,
    clippy::useless_let_if_seq,
    clippy::collapsible_if
)]
pub(super) fn cursor_detail(db_path: &Path, session_id: &str) -> Result<LoadedSession> {
    use rusqlite::{Connection, OpenFlags, params};

    let mut messages = Vec::new();
    if !db_path.is_file() {
        return Ok(LoadedSession {
            tokens: None,
            cost_usd: None,
            messages,
        });
    }

    let Ok(connection) = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return Ok(LoadedSession {
            tokens: None,
            cost_usd: None,
            messages,
        });
    };

    let key = format!("composerData:{session_id}");
    let mut json_str: Option<String> = None;

    if sqlite_table_exists(&connection, "cursorDiskKV")
        && let Ok(mut stmt) =
            connection.prepare("SELECT CAST(value AS TEXT) FROM cursorDiskKV WHERE key = ?1")
        && let Ok(mut rows) = stmt.query(params![key])
        && let Ok(Some(row)) = rows.next()
    {
        json_str = row.get::<_, String>(0).ok();
    }

    if json_str.is_none()
        && sqlite_table_exists(&connection, "ItemTable")
        && let Ok(mut stmt) =
            connection.prepare("SELECT CAST(value AS TEXT) FROM ItemTable WHERE key = ?1")
        && let Ok(mut rows) = stmt.query(params![key])
        && let Ok(Some(row)) = rows.next()
    {
        json_str = row.get::<_, String>(0).ok();
    }

    if let Some(json_str) = json_str {
        if let Ok(val) = serde_json::from_str::<Value>(&json_str) {
            if let Some(conversation) = val.get("conversation").and_then(Value::as_array) {
                for item in conversation {
                    let msg_type = item.get("type").and_then(Value::as_u64).unwrap_or(0);
                    let role = if msg_type == 1 { "user" } else { "assistant" };
                    let content = extract_cursor_message_content(item);
                    if !content.trim().is_empty() {
                        messages.push(SessionMessage {
                            kind: SessionMessageKind::from_provider_role(role),
                            timestamp: string_at(item, "/createdAt"),
                            model: string_at(item, "/model"),
                            metrics: SessionMessageMetrics::default(),
                            content,
                        });
                    }
                }
            }

            if messages.is_empty() {
                if let Some(headers) = val
                    .get("fullConversationHeadersOnly")
                    .and_then(Value::as_array)
                {
                    for head in headers {
                        let msg_type = head.get("type").and_then(Value::as_u64).unwrap_or(0);
                        let role = if msg_type == 1 { "user" } else { "assistant" };
                        let timestamp = string_at(head, "/createdAt");
                        if let Some(bubble_id) = string_at(head, "/bubbleId") {
                            let bubble_key = format!("bubbleId:{session_id}:{bubble_id}");
                            if let Some(bubble_val) =
                                load_cursor_bubble_value(&connection, &bubble_key)
                            {
                                let content = extract_cursor_message_content(&bubble_val);
                                let model = string_at(&bubble_val, "/model");
                                if !content.trim().is_empty() {
                                    messages.push(SessionMessage {
                                        kind: SessionMessageKind::from_provider_role(role),
                                        timestamp,
                                        model,
                                        metrics: SessionMessageMetrics::default(),
                                        content,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(LoadedSession {
        tokens: None,
        cost_usd: None,
        messages,
    })
}

fn sqlite_table_exists(connection: &rusqlite::Connection, table: &str) -> bool {
    use rusqlite::params;
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            params![table],
            |row| row.get(0),
        )
        .unwrap_or(false)
}

fn extract_cursor_message_content(item: &Value) -> String {
    if let Some(text) = item.get("text").and_then(Value::as_str)
        && !text.is_empty()
    {
        return text.to_owned();
    }
    if let Some(rich) = item.get("richText").and_then(Value::as_str)
        && let Ok(rich_val) = serde_json::from_str::<Value>(rich)
    {
        let mut extracted = String::new();
        collect_lexical_text(&rich_val, &mut extracted);
        if !extracted.is_empty() {
            return extracted;
        }
    }
    String::new()
}

fn collect_lexical_text(val: &Value, out: &mut String) {
    if let Some(text) = val.get("text").and_then(Value::as_str) {
        out.push_str(text);
    }
    if let Some(children) = val.get("children").and_then(Value::as_array) {
        for child in children {
            collect_lexical_text(child, out);
        }
    }
}
#[allow(clippy::collapsible_if)]
fn load_cursor_bubble_value(connection: &rusqlite::Connection, bubble_key: &str) -> Option<Value> {
    use rusqlite::params;
    let mut bubble_json = None;
    if sqlite_table_exists(connection, "cursorDiskKV") {
        if let Ok(mut stmt) =
            connection.prepare("SELECT CAST(value AS TEXT) FROM cursorDiskKV WHERE key = ?1")
        {
            if let Ok(mut rows) = stmt.query(params![bubble_key]) {
                if let Ok(Some(row)) = rows.next() {
                    bubble_json = row.get::<_, String>(0).ok();
                }
            }
        }
    }
    if bubble_json.is_none() && sqlite_table_exists(connection, "ItemTable") {
        if let Ok(mut stmt) =
            connection.prepare("SELECT CAST(value AS TEXT) FROM ItemTable WHERE key = ?1")
        {
            if let Ok(mut rows) = stmt.query(params![bubble_key]) {
                if let Ok(Some(row)) = rows.next() {
                    bubble_json = row.get::<_, String>(0).ok();
                }
            }
        }
    }
    bubble_json.and_then(|str_val| serde_json::from_str::<Value>(&str_val).ok())
}
