use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Result, bail};
use serde_json::Value;

use super::{
    AgentSession, MAX_RECORD_BYTES, SessionDetail, SessionMessage, SessionMessageKind,
    SessionMessageMetrics, content_text_full, file_stem, files_with_extension, read_json_file,
    string_at, visit_bounded_lines,
};
use crate::AgentKind;

type Usage = (Option<u64>, Option<f64>);

#[derive(Default)]
struct LoadedSession {
    tokens: Option<u64>,
    cost_usd: Option<f64>,
    messages: Vec<SessionMessage>,
}

pub(super) fn usage(home: &Path, session: &AgentSession) -> Result<Usage> {
    match session.kind {
        AgentKind::Codex | AgentKind::ClaudeCode | AgentKind::Pi | AgentKind::OhMyPi => {
            jsonl_usage(&session.path, &session.kind)
        }
        AgentKind::GeminiCli => {
            let usage = read_json_file(&session.path)?
                .as_ref()
                .map_or((None, None), gemini_usage);
            Ok(usage)
        }
        AgentKind::OpenCode => opencode_usage(home, &session.id),
        AgentKind::Cursor | AgentKind::Custom(_) => Ok((None, None)),
    }
}

pub(super) fn load(home: &Path, selected: &AgentSession) -> Result<SessionDetail> {
    let loaded = match selected.kind {
        AgentKind::Codex => codex_detail(&selected.path)?,
        AgentKind::ClaudeCode | AgentKind::Pi | AgentKind::OhMyPi => {
            nested_jsonl_detail(&selected.path, &selected.kind)?
        }
        AgentKind::GeminiCli => gemini_detail(&selected.path)?,
        AgentKind::OpenCode => opencode_detail(home, &selected.id)?,
        AgentKind::Cursor | AgentKind::Custom(_) => LoadedSession::default(),
    };
    let mut session = selected.clone();
    session.tokens = loaded.tokens;
    session.cost_usd = loaded.cost_usd;
    Ok(SessionDetail {
        session,
        messages: loaded.messages,
    })
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
                    && let Some(tokens) = message_usage_tokens(usage)
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

fn jsonl_usage(path: &Path, kind: &AgentKind) -> Result<Usage> {
    let mut usage = JsonlUsage::new(kind);
    let skipped = visit_bounded_lines(path, |line| {
        if let Ok(record) = serde_json::from_slice::<Value>(line) {
            usage.ingest(&record);
        }
    })?;
    Ok(usage.finish(!skipped))
}

fn codex_detail(path: &Path) -> Result<LoadedSession> {
    let mut usage = JsonlUsage::new(&AgentKind::Codex);
    let mut messages: Vec<SessionMessage> = Vec::new();
    let mut current_model = None;
    let mut pending_assistant: Option<usize> = None;
    let mut turn_last_assistant: Option<usize> = None;
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
        if payload_type == "task_started" {
            pending_assistant = None;
            turn_last_assistant = None;
        } else if payload_type == "token_count" {
            if let Some(index) = pending_assistant.take()
                && let Some(tokens) = payload
                    .pointer("/info/last_token_usage")
                    .and_then(message_usage_tokens)
            {
                messages[index].metrics.tokens = Some(tokens);
            }
        } else if payload_type == "task_complete"
            && let Some(index) = turn_last_assistant
            && let Some(duration_ms) = payload.get("duration_ms").and_then(number_as_milliseconds)
        {
            messages[index].metrics.duration_ms = Some(duration_ms);
        }
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

fn nested_jsonl_detail(path: &Path, kind: &AgentKind) -> Result<LoadedSession> {
    let mut usage = JsonlUsage::new(kind);
    let mut messages: Vec<SessionMessage> = Vec::new();
    let mut metric_targets = BTreeMap::new();
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
        let mut parsed = messages_from_content(
            SessionMessageKind::from_provider_role(role),
            timestamp,
            model,
            content,
        );
        if let Some(index) = parsed
            .iter()
            .rposition(|message| message.kind == SessionMessageKind::Assistant)
        {
            let target = messages.len() + index;
            if let Some(message_id) = string_at(&record, "/message/id")
                && let Some(previous) = metric_targets.insert(message_id, target)
            {
                messages[previous].metrics = SessionMessageMetrics::default();
            }
            parsed[index].metrics.tokens = record
                .pointer("/message/usage")
                .and_then(message_usage_tokens);
            parsed[index].metrics.duration_ms = record
                .pointer("/message/duration")
                .and_then(number_as_milliseconds);
        }
        messages.extend(parsed);
    })?;
    ensure_complete(path, skipped)?;
    let (tokens, cost_usd) = usage.finish(true);
    Ok(LoadedSession {
        tokens,
        cost_usd,
        messages,
    })
}

fn gemini_detail(path: &Path) -> Result<LoadedSession> {
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
        if let Some(index) = parsed
            .iter()
            .rposition(|message| message.kind == SessionMessageKind::Assistant)
        {
            parsed[index].metrics.tokens = message
                .get("tokens")
                .and_then(message_usage_tokens)
                .or_else(|| message.get("usage").and_then(message_usage_tokens));
            parsed[index].metrics.duration_ms = message
                .get("durationMs")
                .and_then(number_as_milliseconds)
                .or_else(|| {
                    message
                        .pointer("/metrics/durationMs")
                        .and_then(number_as_milliseconds)
                });
        }
        messages.extend(parsed);
    }
    messages
}

fn opencode_usage(home: &Path, session_id: &str) -> Result<Usage> {
    let mut usage = NumericUsage::default();
    visit_opencode_messages(home, session_id, |_, message| {
        usage.ingest_opencode(message);
        Ok(())
    })?;
    Ok(usage.finish())
}

fn opencode_detail(home: &Path, session_id: &str) -> Result<LoadedSession> {
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
        for part_path in part_paths {
            let Some(part) = read_json_file(&part_path)? else {
                continue;
            };
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
            parsed[index].metrics.tokens = message.get("tokens").and_then(message_usage_tokens);
            parsed[index].metrics.duration_ms = message
                .pointer("/time/completed")
                .and_then(Value::as_u64)
                .zip(created)
                .map(|(completed, created)| completed.saturating_sub(created));
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

fn messages_from_content(
    parent_kind: SessionMessageKind,
    timestamp: Option<String>,
    model: Option<String>,
    content: &Value,
) -> Vec<SessionMessage> {
    if let Value::Array(parts) = content {
        return parts
            .iter()
            .flat_map(|part| {
                messages_from_content(parent_kind, timestamp.clone(), model.clone(), part)
            })
            .collect();
    }

    let kind =
        content
            .get("type")
            .and_then(Value::as_str)
            .map_or(parent_kind, |value| match value {
                "text" | "input_text" | "output_text" => parent_kind,
                "tool_use" | "tool_call" | "function_call" => tool_call_kind(content),
                "tool_result" | "function_call_output" => SessionMessageKind::ToolResult,
                "error" => SessionMessageKind::Error,
                "thinking" | "reasoning" | "system" | "meta" | "developer_message" => {
                    SessionMessageKind::System
                }
                _ => SessionMessageKind::System,
            });
    content_text_full(content)
        .map(|content| {
            vec![SessionMessage {
                kind,
                timestamp,
                model: (kind == SessionMessageKind::Assistant)
                    .then_some(model)
                    .flatten(),
                metrics: SessionMessageMetrics::default(),
                content,
            }]
        })
        .unwrap_or_default()
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
                metrics: SessionMessageMetrics::default(),
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

fn message_usage_tokens(usage: &Value) -> Option<u64> {
    for pointer in ["/total_tokens", "/totalTokens", "/total"] {
        if let Some(tokens) = usage.pointer(pointer).and_then(Value::as_u64) {
            return Some(tokens);
        }
    }

    let mut total = 0_u64;
    let mut found = false;
    for pointer in [
        "/input_tokens",
        "/output_tokens",
        "/cache_read_input_tokens",
        "/cache_creation_input_tokens",
        "/input",
        "/output",
        "/reasoning",
        "/reasoningTokens",
        "/cache/read",
        "/cache/write",
        "/cacheRead",
        "/cacheWrite",
    ] {
        if let Some(tokens) = usage.pointer(pointer).and_then(Value::as_u64) {
            found = true;
            total = total.saturating_add(tokens);
        }
    }
    found.then_some(total)
}

fn number_as_milliseconds(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_f64()
            .filter(|value| value.is_finite() && *value >= 0.0)
            .and_then(|value| format!("{value:.0}").parse().ok())
    })
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
