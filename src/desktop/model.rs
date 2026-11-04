//! Values crossing the native UI seam. No provider connections or mutable paths.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::session::{AgentSession, SessionDetail, SessionMessage};

#[derive(Debug, Clone, uniffi::Record)]
pub struct DesktopSession {
    pub target: String,
    pub provider: String,
    pub title: String,
    pub project: Option<String>,
    pub updated_at: u64,
    pub started_at: Option<String>,
    pub is_subagent: bool,
    pub parent_target: Option<String>,
    pub active: bool,
    pub tokens: Option<u64>,
    pub cost_usd: Option<f64>,
}

impl DesktopSession {
    pub(super) fn from_session(session: &AgentSession, active: bool) -> Self {
        Self {
            target: session.target(),
            provider: session.kind.slug().to_owned(),
            title: session
                .title
                .clone()
                .unwrap_or_else(|| "未命名会话".to_owned()),
            project: session
                .project
                .as_ref()
                .map(|path| path.display().to_string()),
            updated_at: session.updated_at,
            started_at: session.started_at.clone(),
            is_subagent: session.is_subagent,
            parent_target: session
                .parent_session_id
                .as_ref()
                .map(|id| format!("{}:{id}", session.kind.slug())),
            active,
            tokens: session.tokens,
            cost_usd: session.cost_usd,
        }
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct DesktopProcess {
    pub pid: u32,
    pub provider: String,
    pub project: Option<String>,
    pub runtime_seconds: u64,
    pub memory_bytes: u64,
    pub status: String,
    pub session_target: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct DesktopSnapshot {
    pub sessions: Vec<DesktopSession>,
    pub processes: Vec<DesktopProcess>,
    pub warnings: Vec<String>,
    pub context_providers: Vec<String>,
    pub launch_providers: Vec<String>,
    pub agents: Vec<DesktopAgent>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct DesktopAgent {
    pub provider: String,
    pub name: String,
    pub executable: Option<String>,
    pub homepage: String,
    pub custom: bool,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct DesktopTrashSession {
    pub key: String,
    pub target: String,
    pub provider: String,
    pub title: String,
    pub deleted_at: u64,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct DesktopMessage {
    pub id: String,
    /// Index in the complete transcript, including hidden tool/system records.
    pub index: u64,
    pub role: String,
    pub content: String,
    pub timestamp: Option<String>,
    pub model: Option<String>,
    pub native_fork: bool,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct DesktopSessionDetail {
    pub target: String,
    pub revision: String,
    pub messages: Vec<DesktopMessage>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct DesktopSearchHit {
    pub target: String,
    pub snippet: String,
}

impl DesktopSessionDetail {
    pub(super) fn from_detail(detail: &SessionDetail) -> Self {
        let messages: Vec<_> = detail
            .messages
            .iter()
            .enumerate()
            .map(|(index, message)| DesktopMessage {
                id: message_key(index, message),
                index: index as u64,
                role: message.kind.label().to_owned(),
                content: message.content.clone(),
                timestamp: message.timestamp.clone(),
                model: message.model.clone(),
                native_fork: message.fork_point.is_some(),
            })
            .collect();
        Self {
            target: detail.session.target(),
            revision: detail_revision(detail),
            messages,
        }
    }
}

pub(super) fn message_key(index: usize, message: &SessionMessage) -> String {
    let mut hash = DefaultHasher::new();
    hash_message(message, &mut hash);
    format!("{index}:{:016x}", hash.finish())
}

pub(super) fn detail_revision(detail: &SessionDetail) -> String {
    let mut hash = DefaultHasher::new();
    detail.session.target().hash(&mut hash);
    detail.session.project.hash(&mut hash);
    for message in &detail.messages {
        hash_message(message, &mut hash);
    }
    format!("{:016x}", hash.finish())
}

fn hash_message(message: &SessionMessage, hash: &mut DefaultHasher) {
    message.kind.label().hash(hash);
    message.content.hash(hash);
    message.timestamp.hash(hash);
    message.model.hash(hash);
    message.fork_point.hash(hash);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DesktopResourceKind {
    Skills,
    Mcp,
    Memories,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct DesktopField {
    pub label: String,
    pub value: String,
}

impl DesktopField {
    pub(super) fn new(label: &str, value: impl Into<String>) -> Self {
        Self {
            label: label.to_owned(),
            value: value.into(),
        }
    }
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct DesktopResource {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub scope: String,
    pub description: String,
    pub path: String,
    pub status: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct DesktopResourceDetail {
    pub resource: DesktopResource,
    pub fields: Vec<DesktopField>,
    pub content: String,
    pub probe_status: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DesktopForkMethod {
    Native,
    Context,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct DesktopForkRequest {
    pub target: String,
    pub revision: String,
    pub message_id: String,
    pub method: DesktopForkMethod,
    pub provider: String,
    pub title: String,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct DesktopLaunch {
    pub executable: String,
    pub arguments: Vec<String>,
    pub directory: String,
    pub title: String,
    pub target: Option<String>,
}

#[derive(Debug, Clone, uniffi::Record)]
pub struct DesktopResumePreview {
    pub launch: DesktopLaunch,
    pub unavailable_reason: Option<String>,
}
