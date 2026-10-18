use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::process::{AgentKind, LiveAgent};

mod adapter;
mod relocate;
mod titles;
mod trash;

pub use relocate::{commit_claude_relocation, preview_claude_relocation};
pub use trash::restore_session;

pub use adapter::NativeResumeCommand;

const MAX_SESSION_FILES: usize = 50_000;
const MAX_JSON_BYTES: u64 = 64 * 1_024 * 1_024;
const MAX_RECORD_BYTES: usize = 8 * 1_024 * 1_024;

/// Claude Code's project-directory key. Every character that is not an ASCII
/// letter or digit becomes `-`, which is how Claude names `~/.claude/projects`.
#[must_use]
pub fn claude_project_key(path: &Path) -> Option<String> {
    let text = path.to_str()?;
    if text.is_empty() {
        return None;
    }
    let key: String = text
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect();
    if key.chars().all(|character| character == '-') {
        None
    } else {
        Some(key)
    }
}

/// Whether `updated_at` falls inside the last `days` of 86400-second windows.
/// Zero days means every session.
#[must_use]
pub const fn updated_within_days(updated_at: u64, days: u64, now: u64) -> bool {
    days == 0 || updated_at >= now.saturating_sub(days.saturating_mul(86_400))
}

/// Scan a transcript until the first case-insensitive `query` hit.
///
/// Reads in fixed windows and stops. The returned text is a short slice
/// around that hit, prefixed with a role when one is visible just before it.
pub fn first_transcript_hit(path: &Path, query: &str) -> Result<Option<String>> {
    if query.is_empty() || !path.is_file() {
        return Ok(None);
    }
    let mut file = File::open(path)
        .with_context(|| format!("failed to read agent session {}", path.display()))?;
    let mut carry = Vec::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read agent session {}", path.display()));
            }
        };
        carry.extend_from_slice(&buffer[..read]);
        let text = String::from_utf8_lossy(&carry);
        let lower = text.to_ascii_lowercase();
        if let Some(at) = lower.find(query) {
            return Ok(Some(transcript_snippet(&text, at, query.len())));
        }
        let keep = query.len().saturating_add(240).max(256);
        if carry.len() > keep {
            carry.drain(..carry.len() - keep);
        }
    }
    Ok(None)
}

fn transcript_snippet(text: &str, at: usize, needle: usize) -> String {
    let start = text
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= at.saturating_sub(36))
        .last()
        .unwrap_or(0);
    let end = text
        .char_indices()
        .map(|(index, _)| index)
        .find(|index| *index >= (at + needle + 36).min(text.len()))
        .unwrap_or(text.len());
    let body = text[start..end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let prefix = &text[..at];
    let role = if prefix.contains("\"role\":\"user\"") || prefix.contains("\"type\":\"user\"") {
        "USER"
    } else if prefix.contains("\"role\":\"assistant\"") || prefix.contains("\"type\":\"assistant\"")
    {
        "ASSISTANT"
    } else {
        ""
    };
    if role.is_empty() {
        body
    } else {
        format!("{role}: {body}")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AgentSession {
    pub kind: AgentKind,
    pub id: String,
    pub title: Option<String>,
    pub project: Option<PathBuf>,
    pub path: PathBuf,
    /// Additional native transcript files for the same logical session. A
    /// provider may split one session across several rollout files; `path`
    /// is the most recently updated one and deletion removes every related
    /// file alongside it. The catalog lists each logical session once.
    pub related_paths: BTreeSet<PathBuf>,
    pub started_at: Option<String>,
    pub updated_at: u64,
    pub tokens: Option<u64>,
    pub cost_usd: Option<f64>,
}

impl AgentSession {
    pub(crate) fn target(&self) -> String {
        format!("{}:{}", self.kind.slug(), self.id)
    }

    /// Compact row label for crowded lists: the provider slug plus the first
    /// ID characters, left-aligned for clean tabular display. Details, exports,
    /// clipboard content, and JSON output always keep the full target.
    pub(crate) fn short_target(&self) -> String {
        const SHORT_ID_CHARS: usize = 8;
        let prefix: String = self.id.chars().take(SHORT_ID_CHARS).collect();
        format!("{}:{prefix}", self.kind.slug())
    }
}

/// Native evidence used to establish an exact process-to-session association.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AssociationEvidence {
    /// Provider-owned runtime metadata containing the live PID and session identity.
    NativeRuntime,
    /// The live process has the provider-owned transcript file open.
    OpenSessionFile,
    /// A provider-native resume/session selector in the process argv.
    ResumeArgument,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum AssociationDecision<'a> {
    Exact {
        session: &'a AgentSession,
        evidence: AssociationEvidence,
    },
    ProtectProvider,
    Unsupported,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SessionProtection {
    pub exact_active_targets: BTreeSet<String>,
    pub protected_targets: BTreeSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionMessageKind {
    User,
    Assistant,
    Skill,
    ToolCall,
    ToolResult,
    System,
    Error,
}

impl SessionMessageKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::User => "USER",
            Self::Assistant => "ASSISTANT",
            Self::Skill => "SKILL",
            Self::ToolCall => "TOOL CALL",
            Self::ToolResult => "TOOL RESULT",
            Self::System => "SYSTEM / META",
            Self::Error => "ERROR",
        }
    }

    fn from_provider_role(role: &str) -> Self {
        match role.to_ascii_lowercase().as_str() {
            "user" | "human" => Self::User,
            "assistant" | "gemini" | "model" => Self::Assistant,
            "skill" => Self::Skill,
            "tool_call" | "toolcall" | "tool_use" | "function_call" => Self::ToolCall,
            "tool_result" | "toolresult" | "function_call_output" => Self::ToolResult,
            "error" => Self::Error,
            _ => Self::System,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionMessage {
    pub kind: SessionMessageKind,
    pub timestamp: Option<String>,
    pub model: Option<String>,
    pub metrics: SessionMessageMetrics,
    pub content: String,
}

/// Metrics attached to one persisted message.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionMessageMetrics {
    pub response: Option<ResponseMetrics>,
    pub tool: Option<ToolMetrics>,
}

impl SessionMessageMetrics {
    pub(crate) fn response_mut(&mut self) -> &mut ResponseMetrics {
        self.response.get_or_insert_default()
    }

    pub(crate) fn tool_mut(&mut self) -> &mut ToolMetrics {
        self.tool.get_or_insert_default()
    }
}

/// Exact metrics persisted for one model response.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResponseMetrics {
    pub duration_ms: Option<u64>,
    pub time_to_first_token_ms: Option<u64>,
    pub cost_usd: Option<f64>,
    pub finish_reason: Option<String>,
    pub retry_count: Option<u64>,
    pub error: Option<MetricError>,
    pub tokens: TokenUsage,
}

/// Provider-persisted error details for one model response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricError {
    pub code: Option<String>,
    pub message: String,
}

/// Exact execution metrics persisted for one tool call.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolMetrics {
    pub status: Option<String>,
    pub duration_ms: Option<u64>,
    pub exit_code: Option<i64>,
    pub error: Option<MetricError>,
}

/// Exact token usage persisted for one provider response.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub total: Option<u64>,
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_write: Option<u64>,
    pub cache_write_5m: Option<u64>,
    pub cache_write_1h: Option<u64>,
    pub reasoning: Option<u64>,
    pub tool: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionDetail {
    pub session: AgentSession,
    pub messages: Vec<SessionMessage>,
}

/// Which messages a detail view presents. `Conversation` shows only the
/// user/assistant dialogue and hides tool calls, tool results, skills,
/// system, and error messages; `All` shows everything. Metadata and model
/// usage are always shown regardless of scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DetailScope {
    /// All messages (the historical behavior).
    All,
    /// Only user and assistant messages.
    #[default]
    Conversation,
}

impl DetailScope {
    /// Returns `true` when a message of this kind is visible under the scope.
    #[must_use]
    pub const fn keeps(self, kind: SessionMessageKind) -> bool {
        match self {
            Self::All => true,
            Self::Conversation => matches!(
                kind,
                SessionMessageKind::User | SessionMessageKind::Assistant
            ),
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "complete",
            Self::Conversation => "conversation only",
        }
    }
}

impl SessionDetail {
    /// Iterates the messages visible under `scope` without cloning.
    pub fn messages_in(&self, scope: DetailScope) -> impl Iterator<Item = &SessionMessage> {
        self.messages
            .iter()
            .filter(move |message| scope.keeps(message.kind))
    }

    /// Number of messages hidden by `scope` (always 0 for `All`).
    #[must_use]
    pub fn hidden_message_count(&self, scope: DetailScope) -> usize {
        match scope {
            DetailScope::All => 0,
            DetailScope::Conversation => self
                .messages
                .iter()
                .filter(|message| !scope.keeps(message.kind))
                .count(),
        }
    }

    /// Returns exact persisted response metrics grouped by model.
    #[must_use]
    pub fn model_usage(&self) -> Vec<ModelUsageSummary> {
        let mut summaries = BTreeMap::<String, ModelUsageAccumulator>::new();
        for message in &self.messages {
            let Some(response) = message.metrics.response.as_ref() else {
                continue;
            };
            let Some(model) = message
                .model
                .as_deref()
                .map(str::trim)
                .filter(|model| !model.is_empty())
            else {
                continue;
            };
            summaries
                .entry(model.to_owned())
                .or_default()
                .ingest(response);
        }
        summaries
            .into_iter()
            .map(|(model, summary)| summary.finish(model))
            .collect()
    }
}

/// Exact persisted response totals for one model in a session.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelUsageSummary {
    pub model: String,
    pub responses: u64,
    pub duration_ms: Option<u64>,
    pub average_time_to_first_token_ms: Option<u64>,
    pub cost_usd: Option<f64>,
    pub retry_count: Option<u64>,
    pub errors: u64,
    pub tokens: TokenUsage,
}

#[derive(Debug, Default)]
struct ModelUsageAccumulator {
    responses: u64,
    duration_ms: Option<u64>,
    time_to_first_token_ms: Option<u64>,
    time_to_first_token_samples: u64,
    cost_usd: Option<f64>,
    retry_count: Option<u64>,
    errors: u64,
    tokens: TokenUsage,
}

impl ModelUsageAccumulator {
    fn ingest(&mut self, response: &ResponseMetrics) {
        self.responses = self.responses.saturating_add(1);
        add_optional_u64(&mut self.duration_ms, response.duration_ms);
        if response.time_to_first_token_ms.is_some() {
            self.time_to_first_token_samples = self.time_to_first_token_samples.saturating_add(1);
            add_optional_u64(
                &mut self.time_to_first_token_ms,
                response.time_to_first_token_ms,
            );
        }
        if let Some(cost) = response.cost_usd {
            self.cost_usd = Some(self.cost_usd.unwrap_or_default() + cost);
        }
        add_optional_u64(&mut self.retry_count, response.retry_count);
        self.errors = self
            .errors
            .saturating_add(u64::from(response.error.is_some()));
        self.tokens.add(response.tokens);
    }

    fn finish(self, model: String) -> ModelUsageSummary {
        ModelUsageSummary {
            model,
            responses: self.responses,
            duration_ms: self.duration_ms,
            average_time_to_first_token_ms: self.time_to_first_token_ms.map(|total| {
                total
                    .checked_div(self.time_to_first_token_samples)
                    .unwrap_or_default()
            }),
            cost_usd: self.cost_usd,
            retry_count: self.retry_count,
            errors: self.errors,
            tokens: self.tokens,
        }
    }
}

impl TokenUsage {
    fn add(&mut self, other: Self) {
        add_optional_u64(&mut self.total, other.total);
        add_optional_u64(&mut self.input, other.input);
        add_optional_u64(&mut self.output, other.output);
        add_optional_u64(&mut self.cache_read, other.cache_read);
        add_optional_u64(&mut self.cache_write, other.cache_write);
        add_optional_u64(&mut self.cache_write_5m, other.cache_write_5m);
        add_optional_u64(&mut self.cache_write_1h, other.cache_write_1h);
        add_optional_u64(&mut self.reasoning, other.reasoning);
        add_optional_u64(&mut self.tool, other.tool);
    }
}

fn add_optional_u64(total: &mut Option<u64>, value: Option<u64>) {
    if let Some(value) = value {
        *total = Some(total.unwrap_or_default().saturating_add(value));
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeletionSummary {
    pub files: usize,
    pub directories: usize,
    pub index_records: usize,
}

/// Provider slugs accepted by session filters, derived from the built-in
/// session catalog so error guidance cannot drift from discovery support.
pub fn session_provider_slugs() -> Vec<String> {
    adapter::ProviderAdapter::SESSION_CATALOG
        .iter()
        .map(|adapter| adapter.kind().slug().to_owned())
        .collect()
}

#[derive(Debug, Default)]
pub struct SessionCatalog {
    home: PathBuf,
    overlay_path: PathBuf,
    sessions: Vec<AgentSession>,
    native_titles: BTreeMap<String, Option<String>>,
    warnings: Vec<String>,
}

impl SessionCatalog {
    #[cfg(test)]
    pub fn scan(home: &Path) -> Result<Self> {
        Self::scan_provider(home, None, false, false)
    }

    /// Scan provider-owned session storage, optionally keeping messageless
    /// empty draft sessions that discovery hides by default.
    ///
    /// After discovery, mena-owned display-title overlays replace native
    /// titles for matching `provider:session-id` keys. Overlay files live
    /// under the mena config directory for the current user home.
    pub fn scan_provider(
        home: &Path,
        provider: Option<&AgentKind>,
        include_empty: bool,
        include_archived: bool,
    ) -> Result<Self> {
        let overlay_path = titles::path_for(home);
        let mut sessions = Vec::new();
        let mut warnings = Vec::new();
        for adapter in adapter::ProviderAdapter::SESSION_CATALOG {
            if provider.is_none_or(|kind| adapter.matches(kind)) {
                adapter.discover(
                    home,
                    include_empty,
                    include_archived,
                    &mut sessions,
                    &mut warnings,
                )?;
            }
        }
        sessions.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| left.id.cmp(&right.id))
        });
        let mut sessions = Self::dedupe_logical_sessions(sessions);
        let native_titles = sessions
            .iter()
            .map(|session| (session.target(), session.title.clone()))
            .collect();
        titles::apply(&mut sessions, &titles::load(&overlay_path)?);
        Ok(Self {
            home: home.to_path_buf(),
            overlay_path,
            sessions,
            native_titles,
            warnings,
        })
    }

    pub fn sessions(&self) -> &[AgentSession] {
        &self.sessions
    }

    /// Directories discovery skipped because they could not be read.
    /// A missing directory is not a warning; an unreadable one is.
    #[must_use]
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Collapse native records describing the same logical session into one
    /// catalog row: inputs are sorted newest-first, so the first row for a
    /// `(kind, id)` becomes the primary entry and every additional native
    /// file joins its `related_paths`. The catalog never lists duplicate
    /// rows, while deletion and live association still cover every file.
    fn dedupe_logical_sessions(sessions: Vec<AgentSession>) -> Vec<AgentSession> {
        let mut primary_index: BTreeMap<(AgentKind, String), usize> = BTreeMap::new();
        let mut merged: Vec<AgentSession> = Vec::with_capacity(sessions.len());
        for session in sessions {
            let key = (session.kind.clone(), session.id.clone());
            if let Some(&position) = primary_index.get(&key) {
                let primary = &mut merged[position];
                if session.path != primary.path {
                    primary.related_paths.insert(session.path);
                }
            } else {
                primary_index.insert(key, merged.len());
                merged.push(session);
            }
        }
        merged
    }

    pub fn resolve(&self, provider: Option<&str>, session_id: &str) -> Result<&AgentSession> {
        let mut matches = self.sessions.iter().filter(|session| {
            provider.is_none_or(|name| session.kind.slug() == name)
                && (session.id == session_id || session.id.starts_with(session_id))
        });
        let first = matches.next().with_context(|| {
            let prefix = provider.map_or_else(String::new, |name| format!("{name}:"));
            format!("agent session not found: {prefix}{session_id}")
        })?;
        if matches.any(|candidate| candidate.kind != first.kind || candidate.id != first.id) {
            bail!(
                "agent session selector is ambiguous: {session_id}; use provider:full-session-id"
            );
        }
        Ok(first)
    }

    pub(crate) fn protection(&self, agents: &[LiveAgent]) -> Result<SessionProtection> {
        let mut protection = SessionProtection::default();

        for agent in agents {
            match self.association_for_process(agent)? {
                AssociationDecision::Exact { session, evidence } => {
                    debug_assert_ne!(evidence, AssociationEvidence::ResumeArgument);
                    let target = session.target();
                    protection.exact_active_targets.insert(target.clone());
                    protection.protected_targets.insert(target);
                }
                AssociationDecision::ProtectProvider => {
                    protect_provider_sessions(
                        &self.sessions,
                        &agent.kind,
                        &mut protection.protected_targets,
                    );
                }
                AssociationDecision::Unsupported => {}
            }
        }
        Ok(protection)
    }

    fn association_for_process<'a>(&'a self, agent: &LiveAgent) -> Result<AssociationDecision<'a>> {
        let Some(adapter) = adapter::ProviderAdapter::from_kind(&agent.kind) else {
            return Ok(AssociationDecision::Unsupported);
        };

        let evidence = adapter.process_evidence(&self.home, &agent.process)?;
        let mut candidates = BTreeMap::<String, (&AgentSession, AssociationEvidence)>::new();
        for item in evidence {
            for session in self.sessions.iter().filter(|session| {
                session.kind == agent.kind
                    && item.matches(session)
                    && association_project_consistent(session, agent.process.cwd.as_deref())
            }) {
                candidates
                    .entry(session.target())
                    .and_modify(|(selected, source)| {
                        if session.updated_at > selected.updated_at {
                            *selected = session;
                        }
                        *source = item.source;
                    })
                    .or_insert((session, item.source));
            }
        }

        let decision = match candidates.len() {
            1 => {
                let (_, (session, evidence)) =
                    candidates.into_iter().next().expect("one candidate");
                if evidence == AssociationEvidence::ResumeArgument {
                    AssociationDecision::ProtectProvider
                } else {
                    AssociationDecision::Exact { session, evidence }
                }
            }
            _ => AssociationDecision::ProtectProvider,
        };
        Ok(decision)
    }

    pub fn detail(&self, selected: &AgentSession) -> Result<SessionDetail> {
        adapter::ProviderAdapter::from_kind(&selected.kind).map_or_else(
            || {
                Ok(SessionDetail {
                    session: selected.clone(),
                    messages: Vec::new(),
                })
            },
            |adapter| adapter.load(&self.home, selected),
        )
    }

    pub fn delete_session(&self, selected: &AgentSession) -> Result<DeletionSummary> {
        if !self.sessions.iter().any(|session| {
            session.kind == selected.kind
                && session.id == selected.id
                && session.path == selected.path
        }) {
            bail!("refusing to delete a session that is not in the current catalog");
        }
        let files: BTreeSet<PathBuf> = self
            .sessions
            .iter()
            .filter(|session| session.kind == selected.kind && session.id == selected.id)
            .flat_map(|session| {
                std::iter::once(session.path.clone()).chain(session.related_paths.iter().cloned())
            })
            .collect();
        let adapter = adapter::ProviderAdapter::from_kind(&selected.kind)
            .with_context(|| format!("{} sessions do not support local deletion", selected.kind))?;
        let target = selected.target();
        let summary = adapter.delete(&self.home, selected, files)?;
        titles::put(&self.overlay_path, &target, None).with_context(|| {
            format!(
                "deleted native session files but failed to clear the title overlay for {target}"
            )
        })?;
        Ok(summary)
    }

    /// Set a mena-owned display title for one cataloged session.
    ///
    /// Whitespace-only titles remove the overlay and restore the
    /// provider-native title. Native session files are not modified.
    ///
    /// # Errors
    ///
    /// Returns an error when the session is not in the current catalog, the
    /// title exceeds the length limit, or the overlay file cannot be written.
    pub fn set_title(&self, selected: &AgentSession, title: &str) -> Result<Option<String>> {
        if !self.sessions.iter().any(|session| {
            session.kind == selected.kind
                && session.id == selected.id
                && session.path == selected.path
        }) {
            bail!("refusing to rename a session that is not in the current catalog");
        }
        let normalized = titles::normalize(title)?;
        let target = selected.target();
        titles::put(&self.overlay_path, &target, normalized.clone())?;
        Ok(normalized.or_else(|| self.native_titles.get(&target).cloned().flatten()))
    }
}

fn protect_provider_sessions(
    sessions: &[AgentSession],
    kind: &AgentKind,
    protected_targets: &mut BTreeSet<String>,
) {
    protected_targets.extend(
        sessions
            .iter()
            .filter(|session| &session.kind == kind)
            .map(AgentSession::target),
    );
}

fn association_project_consistent(session: &AgentSession, process_cwd: Option<&Path>) -> bool {
    match (
        session.project.as_deref().filter(|path| !is_root(path)),
        process_cwd.filter(|path| !is_root(path)),
    ) {
        (Some(session_project), Some(process_project)) => {
            paths_equivalent(session_project, process_project)
        }
        _ => true,
    }
}

pub fn paths_equivalent(left: &Path, right: &Path) -> bool {
    left == right
        || fs::canonicalize(left)
            .ok()
            .zip(fs::canonicalize(right).ok())
            .is_some_and(|(left, right)| left == right)
}

#[cfg(test)]
thread_local! {
    static TEST_GROK_HOME: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn test_grok_home() -> Option<PathBuf> {
    TEST_GROK_HOME.with(|slot| slot.borrow().clone())
}

#[cfg(test)]
fn set_test_grok_home(path: Option<PathBuf>) {
    TEST_GROK_HOME.with(|slot| *slot.borrow_mut() = path);
}

#[cfg(test)]
struct TestGrokHomeGuard;

#[cfg(test)]
impl Drop for TestGrokHomeGuard {
    fn drop(&mut self) {
        set_test_grok_home(None);
    }
}

pub fn native_resume_command(kind: &AgentKind, id: &str) -> Result<NativeResumeCommand> {
    let adapter = adapter::ProviderAdapter::from_kind(kind)
        .with_context(|| format!("custom agent {kind} does not define a resume command"))?;
    Ok(adapter.resume_command(id))
}

fn validate_deletion_targets(
    roots: &[PathBuf],
    files: &BTreeSet<PathBuf>,
    directories: &BTreeSet<PathBuf>,
) -> Result<()> {
    for target in files.iter().chain(directories) {
        let Some(root) = roots.iter().find(|root| target.starts_with(root)) else {
            bail!(
                "refusing to delete session artifact outside its provider store: {}",
                target.display()
            );
        };
        let metadata = match fs::symlink_metadata(target) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to inspect session artifact {}", target.display())
                });
            }
        };
        let canonical_root = fs::canonicalize(root)
            .with_context(|| format!("failed to resolve provider store root {}", root.display()))?;
        let resolved = if metadata.file_type().is_symlink() {
            fs::canonicalize(target.parent().unwrap_or(root))
        } else {
            fs::canonicalize(target)
        }
        .with_context(|| format!("failed to resolve session artifact {}", target.display()))?;
        if !resolved.starts_with(&canonical_root) {
            bail!(
                "refusing to delete session artifact through a path outside its provider store: {}",
                target.display()
            );
        }
    }
    Ok(())
}

fn validate_storage_identifier(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        bail!("refusing to use unsafe {label} in a deletion path: {value:?}");
    }
    Ok(())
}

fn remove_file_if_present(
    path: &Path,
    summary: &mut DeletionSummary,
    trash: &mut trash::TrashBatch,
) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect session artifact {}", path.display()));
        }
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        return remove_tree_if_present(path, summary, trash);
    }
    trash.keep_file(path)?;
    summary.files += 1;
    Ok(())
}

fn remove_tree_if_present(
    path: &Path,
    summary: &mut DeletionSummary,
    trash: &mut trash::TrashBatch,
) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect session artifact {}", path.display()));
        }
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return remove_file_if_present(path, summary, trash);
    }
    for entry in fs::read_dir(path)
        .with_context(|| format!("failed to inspect session directory {}", path.display()))?
    {
        let child = entry
            .with_context(|| format!("failed to inspect a session artifact in {}", path.display()))?
            .path();
        let child_metadata = fs::symlink_metadata(&child)
            .with_context(|| format!("failed to inspect session artifact {}", child.display()))?;
        if child_metadata.is_dir() && !child_metadata.file_type().is_symlink() {
            remove_tree_if_present(&child, summary, trash)?;
        } else {
            remove_file_if_present(&child, summary, trash)?;
        }
    }
    fs::remove_dir(path).with_context(|| {
        format!(
            "failed to remove empty session directory {}",
            path.display()
        )
    })?;
    summary.directories += 1;
    Ok(())
}

fn remove_jsonl_records(path: &Path, pointer: &str, expected: &str) -> Result<usize> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect session index {}", path.display()));
        }
    };
    if metadata.len() > MAX_JSON_BYTES {
        bail!(
            "refusing to rewrite oversized session index {} ({} bytes)",
            path.display(),
            metadata.len()
        );
    }
    let input = fs::read(path)
        .with_context(|| format!("failed to read session index {}", path.display()))?;
    let mut output = Vec::with_capacity(input.len());
    let mut removed = 0_usize;
    for line in input.split_inclusive(|byte| *byte == b'\n') {
        let record = line.strip_suffix(b"\n").unwrap_or(line);
        let record = record.strip_suffix(b"\r").unwrap_or(record);
        let matches = serde_json::from_slice::<Value>(record)
            .is_ok_and(|value| value.pointer(pointer).and_then(Value::as_str) == Some(expected));
        if matches {
            removed += 1;
        } else {
            output.extend_from_slice(line);
        }
    }
    if removed > 0 {
        crate::fs::atomic_write(path, &output)
            .with_context(|| format!("failed to update session index {}", path.display()))?;
    }
    Ok(removed)
}

fn content_preview(value: &Value) -> Option<String> {
    let mut parts = Vec::new();
    collect_content_text(value, &mut parts);
    normalize_preview(&parts.join(" "))
}

fn collect_content_text(value: &Value, parts: &mut Vec<String>) {
    match value {
        Value::String(text) => parts.push(text.clone()),
        Value::Array(values) => {
            for value in values {
                collect_content_text(value, parts);
            }
        }
        Value::Object(object) => {
            if let Some(text) = object.get("text").or_else(|| object.get("content")) {
                collect_content_text(text, parts);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn content_text_full(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => (!text.is_empty()).then(|| text.clone()),
        Value::Array(values) => {
            let parts: Vec<_> = values.iter().filter_map(content_text_full).collect();
            (!parts.is_empty()).then(|| parts.join("\n"))
        }
        Value::Object(object) => {
            let kind = object.get("type").and_then(Value::as_str);
            if matches!(kind, Some("text" | "input_text" | "output_text")) {
                return object
                    .get("text")
                    .or_else(|| object.get("content"))
                    .and_then(content_text_full);
            }
            if object.len() == 1
                && let Some(content) = object.get("text").or_else(|| object.get("content"))
            {
                return content_text_full(content);
            }
            serde_json::to_string_pretty(value).ok()
        }
        Value::Bool(_) | Value::Number(_) => Some(value.to_string()),
        Value::Null => None,
    }
}

fn normalize_preview(value: &str) -> Option<String> {
    const MAX_CHARS: usize = 120;
    let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return None;
    }
    let mut chars = normalized.chars();
    let preview: String = chars.by_ref().take(MAX_CHARS).collect();
    Some(if chars.next().is_some() {
        format!(
            "{}…",
            preview.chars().take(MAX_CHARS - 1).collect::<String>()
        )
    } else {
        preview
    })
}

fn is_root(path: &Path) -> bool {
    path.parent().is_none()
}

#[allow(clippy::too_many_arguments)]
fn session(
    kind: AgentKind,
    id: String,
    title: Option<String>,
    project: Option<PathBuf>,
    path: PathBuf,
    started_at: Option<String>,
    tokens: Option<u64>,
    cost_usd: Option<f64>,
) -> Result<AgentSession> {
    Ok(AgentSession {
        kind,
        id,
        title,
        project,
        updated_at: modified_seconds(&path)?,
        path,
        related_paths: BTreeSet::new(),
        started_at,
        tokens,
        cost_usd,
    })
}

struct SessionWalk {
    files: Vec<PathBuf>,
    warnings: Vec<String>,
}

/// Discovery walk. An unreadable directory is reported and skipped so one
/// protected folder cannot hide every other session. Detail loading keeps
/// the strict [`files_with_extension`] path.
fn explained(error: impl Into<anyhow::Error>, message: String) -> anyhow::Error {
    error.into().context(message)
}

fn skip_or_fail(
    strict: bool,
    warnings: &mut Vec<String>,
    notice: String,
    error: anyhow::Error,
) -> Result<()> {
    if strict {
        Err(error)
    } else {
        warnings.push(notice);
        Ok(())
    }
}

#[allow(clippy::too_many_lines)]
fn walk_session_files(root: &Path, extension: &str, strict: bool) -> Result<SessionWalk> {
    let mut walked = SessionWalk {
        files: Vec::new(),
        warnings: Vec::new(),
    };
    match fs::metadata(root) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(walked),
        Err(error) => {
            skip_or_fail(
                strict,
                &mut walked.warnings,
                format!(
                    "skipped unreadable session directory {}: {error}",
                    root.display()
                ),
                explained(
                    error,
                    format!("failed to inspect agent sessions in {}", root.display()),
                ),
            )?;
            return Ok(walked);
        }
        Ok(metadata) if !metadata.is_dir() => {
            skip_or_fail(
                strict,
                &mut walked.warnings,
                format!(
                    "skipped session path that is not a directory: {}",
                    root.display()
                ),
                anyhow::anyhow!("agent session path is not a directory: {}", root.display()),
            )?;
            return Ok(walked);
        }
        Ok(_) => {}
    }
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) => {
                skip_or_fail(
                    strict,
                    &mut walked.warnings,
                    format!(
                        "skipped unreadable session directory {}: {error}",
                        directory.display()
                    ),
                    explained(
                        error,
                        format!("failed to read agent sessions in {}", directory.display()),
                    ),
                )?;
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    skip_or_fail(
                        strict,
                        &mut walked.warnings,
                        format!(
                            "skipped an unreadable session entry in {}: {error}",
                            directory.display()
                        ),
                        explained(
                            error,
                            format!("failed to read an agent session in {}", directory.display()),
                        ),
                    )?;
                    continue;
                }
            };
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => {
                    let path = entry.path();
                    skip_or_fail(
                        strict,
                        &mut walked.warnings,
                        format!("skipped session path {}: {error}", path.display()),
                        explained(
                            error,
                            format!("failed to inspect agent session path {}", path.display()),
                        ),
                    )?;
                    continue;
                }
            };
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file()
                && entry.path().extension().and_then(|value| value.to_str()) == Some(extension)
            {
                walked.files.push(entry.path());
                if walked.files.len() > MAX_SESSION_FILES {
                    bail!(
                        "agent session scan exceeded {MAX_SESSION_FILES} files under {}",
                        root.display()
                    );
                }
            }
        }
    }
    walked.files.sort();
    Ok(walked)
}

fn files_with_extension(root: &Path, extension: &str) -> Result<Vec<PathBuf>> {
    Ok(walk_session_files(root, extension, true)?.files)
}

fn session_files(root: &Path, extension: &str, warnings: &mut Vec<String>) -> Result<Vec<PathBuf>> {
    let walked = walk_session_files(root, extension, false)?;
    warnings.extend(walked.warnings);
    Ok(walked.files)
}

const DISCOVERY_JSON_BYTES: u64 = 1024 * 1024;

fn read_json_file(path: &Path) -> Result<Option<Value>> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to inspect agent session {}", path.display()))?;
    if metadata.len() > MAX_JSON_BYTES {
        return Ok(None);
    }
    let bytes = fs::read(path)
        .with_context(|| format!("failed to read agent session {}", path.display()))?;
    Ok(serde_json::from_slice(&bytes).ok())
}

/// Read only the prefix of a JSON session during discovery. A complete file
/// under 1 MiB is parsed normally. A larger file contributes `sessionId` and
/// `startTime` when those fields appear in the prefix, and is skipped when
/// they do not.
fn read_discovery_json(path: &Path) -> Result<Option<Value>> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to inspect agent session {}", path.display()))?;
    let mut file = File::open(path)
        .with_context(|| format!("failed to read agent session {}", path.display()))?;
    let limit = usize::try_from(metadata.len().min(DISCOVERY_JSON_BYTES)).unwrap_or(usize::MAX);
    let mut bytes = vec![0; limit];
    let mut filled = 0;
    while filled < bytes.len() {
        match file.read(&mut bytes[filled..]) {
            Ok(0) => break,
            Ok(count) => filled += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read agent session {}", path.display()));
            }
        }
    }
    bytes.truncate(filled);
    if metadata.len() <= DISCOVERY_JSON_BYTES {
        return Ok(serde_json::from_slice(&bytes).ok());
    }
    if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
        return Ok(Some(value));
    }
    Ok(discovery_prefix_value(&bytes))
}

fn discovery_prefix_value(bytes: &[u8]) -> Option<Value> {
    let text = String::from_utf8_lossy(bytes);
    let session_id = json_string_field(&text, "sessionId")?;
    let mut map = serde_json::Map::new();
    map.insert("sessionId".into(), Value::String(session_id));
    if let Some(start) = json_string_field(&text, "startTime") {
        map.insert("startTime".into(), Value::String(start));
    }
    Some(Value::Object(map))
}

fn json_string_field(text: &str, field: &str) -> Option<String> {
    let needle = format!("\"{field}\"");
    let start = text.find(&needle)?;
    let rest = text[start + needle.len()..]
        .trim_start()
        .strip_prefix(':')?
        .trim_start();
    let rest = rest.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = rest.chars();
    while let Some(character) = chars.next() {
        match character {
            '"' => return Some(out),
            '\\' => {
                let escaped = chars.next()?;
                out.push(match escaped {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    '"' | '\\' | '/' => escaped,
                    _ => return None,
                });
            }
            other => out.push(other),
        }
    }
    None
}

fn visit_bounded_lines(path: &Path, mut visitor: impl FnMut(&[u8])) -> Result<bool> {
    visit_bounded_lines_limit(path, None, |line| {
        visitor(line);
        true
    })
}

/// Visit at most `limit` complete lines (`None` for the whole file), with
/// oversized records skipped instead of buffered. The visitor returns `false`
/// to stop early once it has every field it needs, so discovery never parses
/// the bulk of a transcript. Returns whether any record was skipped.
fn visit_bounded_lines_limit(
    path: &Path,
    limit: Option<usize>,
    mut visitor: impl FnMut(&[u8]) -> bool,
) -> Result<bool> {
    let file = File::open(path)
        .with_context(|| format!("failed to read agent session {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut line = Vec::new();
    let mut overflow = false;
    let mut skipped = false;
    let mut visited = 0_usize;

    loop {
        let available = reader
            .fill_buf()
            .with_context(|| format!("failed to read agent session {}", path.display()))?;
        if available.is_empty() {
            if !line.is_empty() && !overflow {
                visitor(trim_carriage_return(&line));
            }
            return Ok(skipped);
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let length = newline.map_or(available.len(), |index| index + 1);
        let content_length = newline.unwrap_or(length);
        if !overflow {
            if line.len().saturating_add(content_length) <= MAX_RECORD_BYTES {
                line.extend_from_slice(&available[..content_length]);
            } else {
                overflow = true;
                skipped = true;
                line.clear();
            }
        }
        reader.consume(length);
        if newline.is_some() {
            if !overflow {
                if !visitor(trim_carriage_return(&line)) {
                    return Ok(skipped);
                }
                visited += 1;
            }
            line.clear();
            overflow = false;
            if limit.is_some_and(|limit| visited >= limit) {
                return Ok(skipped);
            }
        }
    }
}

fn modified_seconds(path: &Path) -> Result<u64> {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
        .duration_since(UNIX_EPOCH)
        .map_or(Ok(0), |duration| Ok(duration.as_secs()))
}

fn string_at(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn file_stem(path: &Path) -> Option<String> {
    path.file_stem()
        .and_then(|value| value.to_str())
        .map(str::to_owned)
}

fn trim_carriage_return(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\r").unwrap_or(line)
}

#[cfg(test)]
mod tests;
