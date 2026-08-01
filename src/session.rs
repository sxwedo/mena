use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags, params};
use serde_json::Value;

use crate::AgentKind;

const MAX_SESSION_FILES: usize = 50_000;
const MAX_JSON_BYTES: u64 = 64 * 1_024 * 1_024;
const MAX_RECORD_BYTES: usize = 8 * 1_024 * 1_024;

#[derive(Debug, Clone, PartialEq)]
pub struct AgentSession {
    pub kind: AgentKind,
    pub id: String,
    pub title: Option<String>,
    pub project: Option<PathBuf>,
    pub path: PathBuf,
    pub started_at: Option<String>,
    pub updated_at: u64,
    pub tokens: Option<u64>,
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionMessageKind {
    User,
    Assistant,
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
            "tool_call" | "tool_use" | "function_call" => Self::ToolCall,
            "tool_result" | "function_call_output" => Self::ToolResult,
            "error" => Self::Error,
            _ => Self::System,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMessage {
    pub kind: SessionMessageKind,
    pub timestamp: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionDetail {
    pub session: AgentSession,
    pub messages: Vec<SessionMessage>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeletionSummary {
    pub files: usize,
    pub directories: usize,
    pub index_records: usize,
}

#[derive(Debug, Default)]
pub struct SessionCatalog {
    home: PathBuf,
    sessions: Vec<AgentSession>,
}

#[derive(Debug, Clone, Copy)]
struct CachedUsage {
    updated_at: u64,
    tokens: Option<u64>,
    cost_usd: Option<f64>,
}

#[derive(Debug, Default)]
pub struct UsageCache {
    entries: BTreeMap<PathBuf, CachedUsage>,
}

impl UsageCache {
    pub fn enrich(
        &mut self,
        catalog: &SessionCatalog,
        session: &AgentSession,
    ) -> Result<AgentSession> {
        if let Some(cached) = self
            .entries
            .get(&session.path)
            .filter(|cached| cached.updated_at == session.updated_at)
        {
            let mut enriched = session.clone();
            enriched.tokens = cached.tokens;
            enriched.cost_usd = cached.cost_usd;
            return Ok(enriched);
        }

        let enriched = catalog.with_usage(session)?;
        self.entries.insert(
            session.path.clone(),
            CachedUsage {
                updated_at: session.updated_at,
                tokens: enriched.tokens,
                cost_usd: enriched.cost_usd,
            },
        );
        Ok(enriched)
    }
}

impl SessionCatalog {
    #[cfg(test)]
    pub fn scan(home: &Path) -> Result<Self> {
        Self::scan_provider(home, None)
    }

    pub fn scan_provider(home: &Path, provider: Option<&AgentKind>) -> Result<Self> {
        let mut sessions = Vec::new();
        if provider.is_none_or(|kind| kind == &AgentKind::Codex) {
            scan_codex(home, &mut sessions)?;
        }
        if provider.is_none_or(|kind| kind == &AgentKind::ClaudeCode) {
            scan_claude(home, &mut sessions)?;
        }
        if provider.is_none_or(|kind| kind == &AgentKind::GeminiCli) {
            scan_gemini(home, &mut sessions)?;
        }
        if provider.is_none_or(|kind| kind == &AgentKind::OpenCode) {
            scan_opencode(home, &mut sessions)?;
        }
        if provider.is_none_or(|kind| kind == &AgentKind::Pi) {
            scan_pi(home, &mut sessions)?;
        }
        if provider.is_none_or(|kind| kind == &AgentKind::OhMyPi) {
            scan_oh_my_pi(home, &mut sessions)?;
        }
        sessions.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(Self {
            home: home.to_path_buf(),
            sessions,
        })
    }

    pub fn sessions(&self) -> &[AgentSession] {
        &self.sessions
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
        if matches.next().is_some() {
            bail!(
                "agent session selector is ambiguous: {session_id}; use provider:full-session-id"
            );
        }
        Ok(first)
    }

    pub fn latest_for_process(
        &self,
        kind: &AgentKind,
        project: Option<&Path>,
        process_started_at: u64,
    ) -> Option<&AgentSession> {
        if let Some(project) = project.filter(|project| !is_root(project)) {
            return self
                .sessions
                .iter()
                .filter(|session| {
                    &session.kind == kind && session.project.as_deref() == Some(project)
                })
                .max_by_key(|session| session.updated_at);
        }
        self.sessions
            .iter()
            .filter(|session| {
                &session.kind == kind && session.updated_at >= process_started_at.saturating_sub(5)
            })
            .max_by_key(|session| session.updated_at)
    }

    pub fn with_usage(&self, session: &AgentSession) -> Result<AgentSession> {
        let mut enriched = session.clone();
        let (tokens, cost_usd) = match session.kind {
            AgentKind::Codex => codex_usage(&session.path)?,
            AgentKind::ClaudeCode => claude_usage(&session.path)?,
            AgentKind::GeminiCli => gemini_usage(&session.path)?,
            AgentKind::OpenCode => opencode_usage(&self.home, &session.id)?,
            AgentKind::Pi | AgentKind::OhMyPi => pi_usage(&session.path)?,
            AgentKind::Cursor | AgentKind::Custom(_) => (None, None),
        };
        enriched.tokens = tokens;
        enriched.cost_usd = cost_usd;
        Ok(enriched)
    }

    pub fn detail(&self, selected: &AgentSession) -> Result<SessionDetail> {
        let session = self.with_usage(selected)?;
        let messages = match session.kind {
            AgentKind::Codex => codex_messages(&session.path)?,
            AgentKind::ClaudeCode | AgentKind::Pi | AgentKind::OhMyPi => {
                nested_jsonl_messages(&session.path)?
            }
            AgentKind::GeminiCli => gemini_messages(&session.path)?,
            AgentKind::OpenCode => opencode_messages(&self.home, &session.id)?,
            AgentKind::Cursor | AgentKind::Custom(_) => Vec::new(),
        };
        Ok(SessionDetail { session, messages })
    }

    pub fn delete_session(&self, selected: &AgentSession) -> Result<DeletionSummary> {
        if !self.sessions.iter().any(|session| {
            session.kind == selected.kind
                && session.id == selected.id
                && session.path == selected.path
        }) {
            bail!("refusing to delete a session that is not in the current catalog");
        }
        validate_storage_identifier(&selected.id, "session ID")?;

        let mut files: BTreeSet<PathBuf> = self
            .sessions
            .iter()
            .filter(|session| session.kind == selected.kind && session.id == selected.id)
            .map(|session| session.path.clone())
            .collect();
        let mut directories = BTreeSet::new();
        collect_provider_artifacts(&self.home, selected, &mut files, &mut directories)?;
        validate_deletion_targets(&self.home, &selected.kind, &files, &directories)?;

        let index_records = delete_provider_index_records(&self.home, selected)?;
        let mut summary = DeletionSummary {
            index_records,
            ..DeletionSummary::default()
        };
        for file in files {
            remove_file_if_present(&file, &mut summary)?;
        }
        let mut directories: Vec<_> = directories.into_iter().collect();
        directories.sort_by_key(|directory| std::cmp::Reverse(directory.components().count()));
        for directory in directories {
            remove_tree_if_present(&directory, &mut summary)?;
        }
        Ok(summary)
    }
}

fn load_codex_titles(home: &Path) -> BTreeMap<String, String> {
    let mut titles = BTreeMap::new();
    let Ok(entries) = fs::read_dir(home.join(".codex")) else {
        return titles;
    };
    let mut databases: Vec<_> = entries
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("state_") && name.ends_with(".sqlite"))
        })
        .collect();
    databases.sort();
    for database in databases {
        let Ok(connection) = Connection::open_with_flags(
            database,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) else {
            continue;
        };
        let Ok(mut statement) = connection.prepare("SELECT id, title FROM threads") else {
            continue;
        };
        let Ok(rows) = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }) else {
            continue;
        };
        for row in rows.flatten() {
            if let Some(title) = normalize_preview(&row.1) {
                titles.insert(row.0, title);
            }
        }
    }
    titles
}

fn scan_codex(home: &Path, sessions: &mut Vec<AgentSession>) -> Result<()> {
    let indexed_titles = load_codex_titles(home);
    for path in files_with_extension(&home.join(".codex/sessions"), "jsonl")? {
        let mut id = None;
        let mut title = None;
        let mut project = None;
        let mut started_at = None;
        visit_bounded_lines_limit(&path, Some(128), |line| {
            let Ok(record) = serde_json::from_slice::<Value>(line) else {
                return;
            };
            if record.get("type").and_then(Value::as_str) == Some("session_meta") {
                id = string_at(&record, "/payload/id");
                project = string_at(&record, "/payload/cwd").map(PathBuf::from);
                started_at = string_at(&record, "/payload/timestamp")
                    .or_else(|| string_at(&record, "/timestamp"));
            }
            if title.is_none()
                && record.pointer("/payload/type").and_then(Value::as_str) == Some("message")
                && record.pointer("/payload/role").and_then(Value::as_str) == Some("user")
            {
                title = record.pointer("/payload/content").and_then(content_preview);
            }
        })?;
        if let Some(id) = id {
            let title = indexed_titles.get(&id).cloned().or(title);
            sessions.push(session(
                AgentKind::Codex,
                id,
                title,
                project,
                path,
                started_at,
                None,
                None,
            )?);
        }
    }
    Ok(())
}

fn scan_claude(home: &Path, sessions: &mut Vec<AgentSession>) -> Result<()> {
    for path in files_with_extension(&home.join(".claude/projects"), "jsonl")? {
        let mut id = None;
        let mut title = None;
        let mut project = None;
        let mut started_at = None;
        visit_bounded_lines_limit(&path, Some(64), |line| {
            let Ok(record) = serde_json::from_slice::<Value>(line) else {
                return;
            };
            id = id.take().or_else(|| string_at(&record, "/sessionId"));
            project = project
                .take()
                .or_else(|| string_at(&record, "/cwd").map(PathBuf::from));
            started_at = started_at
                .take()
                .or_else(|| string_at(&record, "/timestamp"));
            if title.is_none()
                && record.pointer("/message/role").and_then(Value::as_str) == Some("user")
                && record.get("isMeta").and_then(Value::as_bool) != Some(true)
            {
                title = record.pointer("/message/content").and_then(content_preview);
            }
        })?;
        if let Some(id) = id.or_else(|| file_stem(&path)) {
            sessions.push(session(
                AgentKind::ClaudeCode,
                id,
                title,
                project,
                path,
                started_at,
                None,
                None,
            )?);
        }
    }
    Ok(())
}

fn scan_gemini(home: &Path, sessions: &mut Vec<AgentSession>) -> Result<()> {
    let root = home.join(".gemini/tmp");
    for path in files_with_extension(&root, "json")? {
        if path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            != Some("chats")
        {
            continue;
        }
        let Some(value) = read_json_file(&path)? else {
            continue;
        };
        let Some(id) = string_at(&value, "/sessionId") else {
            continue;
        };
        let project = path
            .parent()
            .and_then(Path::parent)
            .map(|directory| directory.join(".project_root"))
            .and_then(|marker| fs::read_to_string(marker).ok())
            .map(|value| PathBuf::from(value.trim()));
        sessions.push(session(
            AgentKind::GeminiCli,
            id,
            value
                .get("messages")
                .and_then(Value::as_array)
                .and_then(|messages| {
                    messages.iter().find_map(|message| {
                        (message.get("type").and_then(Value::as_str) == Some("user"))
                            .then(|| message.get("content").and_then(content_preview))
                            .flatten()
                    })
                }),
            project,
            path,
            string_at(&value, "/startTime"),
            None,
            None,
        )?);
    }
    Ok(())
}

fn scan_opencode(home: &Path, sessions: &mut Vec<AgentSession>) -> Result<()> {
    let root = home.join(".local/share/opencode/storage");
    for path in files_with_extension(&root.join("session"), "json")? {
        let Some(value) = read_json_file(&path)? else {
            continue;
        };
        let Some(id) = string_at(&value, "/id") else {
            continue;
        };
        sessions.push(session(
            AgentKind::OpenCode,
            id,
            string_at(&value, "/title").and_then(|title| normalize_preview(&title)),
            string_at(&value, "/directory").map(PathBuf::from),
            path,
            value
                .pointer("/time/created")
                .and_then(Value::as_u64)
                .map(|time| time.to_string()),
            None,
            None,
        )?);
    }
    Ok(())
}

fn scan_pi(home: &Path, sessions: &mut Vec<AgentSession>) -> Result<()> {
    scan_pi_sessions(&home.join(".pi/agent/sessions"), &AgentKind::Pi, sessions)
}

fn scan_oh_my_pi(home: &Path, sessions: &mut Vec<AgentSession>) -> Result<()> {
    scan_pi_sessions(
        &home.join(".omp/agent/sessions"),
        &AgentKind::OhMyPi,
        sessions,
    )
}

fn scan_pi_sessions(root: &Path, kind: &AgentKind, sessions: &mut Vec<AgentSession>) -> Result<()> {
    for path in files_with_extension(root, "jsonl")? {
        let mut id = None;
        let mut title = None;
        let mut project = None;
        let mut started_at = None;
        visit_bounded_lines_limit(&path, Some(64), |line| {
            let Ok(record) = serde_json::from_slice::<Value>(line) else {
                return;
            };
            if record.get("type").and_then(Value::as_str) == Some("session") {
                id = string_at(&record, "/id");
                project = string_at(&record, "/cwd").map(PathBuf::from);
                started_at = string_at(&record, "/timestamp");
            }
            if title.is_none() && record.get("type").and_then(Value::as_str) == Some("title") {
                title = string_at(&record, "/title").and_then(|title| normalize_preview(&title));
            }
            if title.is_none()
                && record.pointer("/message/role").and_then(Value::as_str) == Some("user")
            {
                title = record.pointer("/message/content").and_then(content_preview);
            }
        })?;
        if let Some(id) = id {
            sessions.push(session(
                kind.clone(),
                id,
                title,
                project,
                path,
                started_at,
                None,
                None,
            )?);
        }
    }
    Ok(())
}

fn collect_provider_artifacts(
    home: &Path,
    session: &AgentSession,
    files: &mut BTreeSet<PathBuf>,
    directories: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    match session.kind {
        AgentKind::Codex => {
            collect_files_with_prefix(
                &home.join(".codex/shell_snapshots"),
                &format!("{}.", session.id),
                files,
            )?;
        }
        AgentKind::ClaudeCode => {
            if let Some(parent) = session.path.parent() {
                directories.insert(parent.join(&session.id));
            }
            for root in ["session-env", "file-history"] {
                directories.insert(home.join(".claude").join(root).join(&session.id));
            }
            collect_claude_team_artifacts(home, session, files, directories)?;
            collect_matching_json_files(
                &home.join(".claude/sessions"),
                "/sessionId",
                &session.id,
                files,
            )?;
            collect_matching_json_files(
                &home.join(".claude/plugins/claude-hud/transcript-cache"),
                "/transcriptPath",
                &session.path.to_string_lossy(),
                files,
            )?;
        }
        AgentKind::OpenCode => {
            let storage = home.join(".local/share/opencode/storage");
            let message_directory = storage.join("message").join(&session.id);
            if message_directory.exists() {
                for entry in fs::read_dir(&message_directory).with_context(|| {
                    format!(
                        "failed to inspect OpenCode messages in {}",
                        message_directory.display()
                    )
                })? {
                    let entry = entry.with_context(|| {
                        format!(
                            "failed to inspect an OpenCode message in {}",
                            message_directory.display()
                        )
                    })?;
                    if entry.file_type().is_ok_and(|file_type| file_type.is_file())
                        && let Some(message_id) = file_stem(&entry.path())
                    {
                        validate_storage_identifier(&message_id, "OpenCode message ID")?;
                        directories.insert(storage.join("part").join(message_id));
                    }
                }
            }
            directories.insert(message_directory);
            files.insert(
                storage
                    .join("session_diff")
                    .join(format!("{}.json", session.id)),
            );
            files.insert(storage.join("todo").join(format!("{}.json", session.id)));
        }
        AgentKind::GeminiCli | AgentKind::Pi | AgentKind::OhMyPi => {}
        AgentKind::Cursor | AgentKind::Custom(_) => {
            bail!("{} sessions do not support local deletion", session.kind)
        }
    }
    Ok(())
}

fn collect_claude_team_artifacts(
    home: &Path,
    session: &AgentSession,
    files: &mut BTreeSet<PathBuf>,
    directories: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    let prefix: String = session.id.chars().take(8).collect();
    let team = home.join(".claude/teams").join(format!("session-{prefix}"));
    let config = team.join("config.json");
    let matches_session = config.exists()
        && read_json_file(&config)?.is_some_and(|value| {
            value.pointer("/leadSessionId").and_then(Value::as_str) == Some(session.id.as_str())
        });
    if !matches_session {
        return Ok(());
    }

    let tasks = home.join(".claude/tasks").join(format!("session-{prefix}"));
    if tasks.exists() {
        for entry in fs::read_dir(&tasks).with_context(|| {
            format!(
                "failed to inspect Claude task artifacts in {}",
                tasks.display()
            )
        })? {
            let path = entry
                .with_context(|| format!("failed to inspect a Claude task in {}", tasks.display()))?
                .path();
            if let Some(agent_id) = file_stem(&path) {
                validate_storage_identifier(&agent_id, "Claude task agent ID")?;
                files.insert(home.join(".claude/debug").join(format!("{agent_id}.txt")));
            }
        }
    }
    directories.insert(tasks);
    directories.insert(team);
    Ok(())
}

fn collect_matching_json_files(
    root: &Path,
    pointer: &str,
    expected: &str,
    files: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    for path in files_with_extension(root, "json")? {
        if read_json_file(&path)?
            .is_some_and(|value| value.pointer(pointer).and_then(Value::as_str) == Some(expected))
        {
            files.insert(path);
        }
    }
    Ok(())
}

fn collect_files_with_prefix(
    root: &Path,
    prefix: &str,
    files: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root)
        .with_context(|| format!("failed to inspect session artifacts in {}", root.display()))?
    {
        let entry = entry.with_context(|| {
            format!("failed to inspect a session artifact in {}", root.display())
        })?;
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with(prefix))
        {
            files.insert(entry.path());
        }
    }
    Ok(())
}

fn validate_deletion_targets(
    home: &Path,
    kind: &AgentKind,
    files: &BTreeSet<PathBuf>,
    directories: &BTreeSet<PathBuf>,
) -> Result<()> {
    let roots = match kind {
        AgentKind::Codex => vec![
            home.join(".codex/sessions"),
            home.join(".codex/shell_snapshots"),
        ],
        AgentKind::ClaudeCode => vec![home.join(".claude")],
        AgentKind::GeminiCli => vec![home.join(".gemini/tmp")],
        AgentKind::OpenCode => vec![home.join(".local/share/opencode/storage")],
        AgentKind::Pi => vec![home.join(".pi/agent/sessions")],
        AgentKind::OhMyPi => vec![home.join(".omp/agent/sessions")],
        AgentKind::Cursor | AgentKind::Custom(_) => Vec::new(),
    };
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

fn remove_file_if_present(path: &Path, summary: &mut DeletionSummary) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect session artifact {}", path.display()));
        }
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        return remove_tree_if_present(path, summary);
    }
    fs::remove_file(path)
        .with_context(|| format!("failed to permanently delete {}", path.display()))?;
    summary.files += 1;
    Ok(())
}

fn remove_tree_if_present(path: &Path, summary: &mut DeletionSummary) -> Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect session artifact {}", path.display()));
        }
    };
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return remove_file_if_present(path, summary);
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
            remove_tree_if_present(&child, summary)?;
        } else {
            remove_file_if_present(&child, summary)?;
        }
    }
    fs::remove_dir(path)
        .with_context(|| format!("failed to permanently delete {}", path.display()))?;
    summary.directories += 1;
    Ok(())
}

fn delete_provider_index_records(home: &Path, session: &AgentSession) -> Result<usize> {
    match session.kind {
        AgentKind::Codex => Ok(delete_codex_database_rows(home, &session.id)?
            + remove_jsonl_records(&home.join(".codex/session_index.jsonl"), "/id", &session.id)?),
        AgentKind::ClaudeCode => remove_jsonl_records(
            &home.join(".claude/history.jsonl"),
            "/sessionId",
            &session.id,
        ),
        AgentKind::GeminiCli
        | AgentKind::OpenCode
        | AgentKind::Pi
        | AgentKind::OhMyPi
        | AgentKind::Cursor
        | AgentKind::Custom(_) => Ok(0),
    }
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

fn delete_codex_database_rows(home: &Path, session_id: &str) -> Result<usize> {
    let codex_root = home.join(".codex");
    if !codex_root.exists() {
        return Ok(0);
    }
    let mut removed = 0_usize;
    for entry in fs::read_dir(&codex_root)
        .with_context(|| format!("failed to inspect Codex state in {}", codex_root.display()))?
    {
        let database = entry
            .with_context(|| format!("failed to inspect Codex state in {}", codex_root.display()))?
            .path();
        let Some(name) = database.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.ends_with(".sqlite")
            || !["state_", "goals_", "logs_", "memories_"]
                .iter()
                .any(|prefix| name.starts_with(prefix))
        {
            continue;
        }
        let mut connection = Connection::open_with_flags(
            &database,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .with_context(|| format!("failed to open Codex state database {}", database.display()))?;
        connection
            .busy_timeout(std::time::Duration::from_secs(2))
            .with_context(|| {
                format!(
                    "failed to configure Codex state database {}",
                    database.display()
                )
            })?;
        let transaction = connection.transaction().with_context(|| {
            format!(
                "failed to update Codex state database {}",
                database.display()
            )
        })?;
        for (table, statement) in codex_delete_statements(name) {
            if sqlite_table_exists(&transaction, table)? {
                removed += transaction
                    .execute(statement, params![session_id])
                    .with_context(|| {
                        format!(
                            "failed to delete session metadata from {table} in {}",
                            database.display()
                        )
                    })?;
            }
        }
        transaction.commit().with_context(|| {
            format!(
                "failed to commit Codex state cleanup in {}",
                database.display()
            )
        })?;
    }
    Ok(removed)
}

fn codex_delete_statements(database_name: &str) -> &'static [(&'static str, &'static str)] {
    if database_name.starts_with("state_") {
        &[
            (
                "thread_dynamic_tools",
                "DELETE FROM thread_dynamic_tools WHERE thread_id = ?1",
            ),
            (
                "thread_spawn_edges",
                "DELETE FROM thread_spawn_edges WHERE parent_thread_id = ?1 OR child_thread_id = ?1",
            ),
            ("threads", "DELETE FROM threads WHERE id = ?1"),
        ]
    } else if database_name.starts_with("goals_") {
        &[
            (
                "thread_goal_continuation_deferrals",
                "DELETE FROM thread_goal_continuation_deferrals WHERE thread_id = ?1",
            ),
            (
                "thread_goals",
                "DELETE FROM thread_goals WHERE thread_id = ?1",
            ),
        ]
    } else if database_name.starts_with("logs_") {
        &[("logs", "DELETE FROM logs WHERE thread_id = ?1")]
    } else if database_name.starts_with("memories_") {
        &[
            (
                "stage1_outputs",
                "DELETE FROM stage1_outputs WHERE thread_id = ?1",
            ),
            ("jobs", "DELETE FROM jobs WHERE job_key = ?1"),
        ]
    } else {
        &[]
    }
}

fn sqlite_table_exists(transaction: &rusqlite::Transaction<'_>, table: &str) -> Result<bool> {
    transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            params![table],
            |row| row.get(0),
        )
        .context("failed to inspect a Codex state database schema")
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
        started_at,
        tokens,
        cost_usd,
    })
}

fn files_with_extension(root: &Path, extension: &str) -> Result<Vec<PathBuf>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory)
            .with_context(|| format!("failed to read agent sessions in {}", directory.display()))?;
        for entry in entries {
            let entry = entry.with_context(|| {
                format!("failed to read an agent session in {}", directory.display())
            })?;
            let file_type = entry.file_type().with_context(|| {
                format!(
                    "failed to inspect agent session path {}",
                    entry.path().display()
                )
            })?;
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file()
                && entry.path().extension().and_then(|value| value.to_str()) == Some(extension)
            {
                files.push(entry.path());
                if files.len() > MAX_SESSION_FILES {
                    bail!(
                        "agent session scan exceeded {MAX_SESSION_FILES} files under {}",
                        root.display()
                    );
                }
            }
        }
    }
    files.sort();
    Ok(files)
}

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

fn visit_bounded_lines(path: &Path, visitor: impl FnMut(&[u8])) -> Result<bool> {
    visit_bounded_lines_limit(path, None, visitor)
}

fn visit_bounded_lines_limit(
    path: &Path,
    limit: Option<usize>,
    mut visitor: impl FnMut(&[u8]),
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
                visitor(trim_carriage_return(&line));
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

fn codex_usage(path: &Path) -> Result<(Option<u64>, Option<f64>)> {
    let mut tokens = None;
    let skipped = visit_bounded_lines(path, |line| {
        let Ok(record) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        if record.pointer("/payload/type").and_then(Value::as_str) == Some("token_count")
            && let Some(total) = record
                .pointer("/payload/info/total_token_usage/total_tokens")
                .and_then(Value::as_u64)
        {
            tokens = Some(total);
        }
    })?;
    Ok(((!skipped).then_some(tokens).flatten(), None))
}

fn codex_messages(path: &Path) -> Result<Vec<SessionMessage>> {
    let mut messages = Vec::new();
    let skipped = visit_bounded_lines(path, |line| {
        let Ok(record) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        let Some(payload) = record.get("payload") else {
            return;
        };
        let Some(payload_type) = payload.get("type").and_then(Value::as_str) else {
            return;
        };
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
                    messages_from_content(
                        SessionMessageKind::from_provider_role(role),
                        timestamp,
                        content,
                    )
                })
                .unwrap_or_default()
        } else {
            let kind = if payload_type == "error" {
                Some(SessionMessageKind::Error)
            } else if payload_type.contains("call_output") || payload_type.ends_with("_result") {
                Some(SessionMessageKind::ToolResult)
            } else if payload_type.ends_with("_call") {
                Some(SessionMessageKind::ToolCall)
            } else {
                Some(SessionMessageKind::System)
            };
            kind.and_then(|kind| {
                content_text_full(payload).map(|content| SessionMessage {
                    kind,
                    timestamp,
                    content,
                })
            })
            .into_iter()
            .collect()
        };
        messages.extend(parsed);
    })?;
    if skipped {
        bail!(
            "cannot show the complete transcript because {} contains a record larger than {MAX_RECORD_BYTES} bytes",
            path.display()
        );
    }
    Ok(messages)
}

fn nested_jsonl_messages(path: &Path) -> Result<Vec<SessionMessage>> {
    let mut messages = Vec::new();
    let skipped = visit_bounded_lines(path, |line| {
        let Ok(record) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        let Some(role) = record.pointer("/message/role").and_then(Value::as_str) else {
            return;
        };
        let Some(content) = record.pointer("/message/content") else {
            return;
        };
        let timestamp =
            string_at(&record, "/timestamp").or_else(|| string_at(&record, "/message/timestamp"));
        messages.extend(messages_from_content(
            SessionMessageKind::from_provider_role(role),
            timestamp,
            content,
        ));
    })?;
    if skipped {
        bail!(
            "cannot show the complete transcript because {} contains a record larger than {MAX_RECORD_BYTES} bytes",
            path.display()
        );
    }
    Ok(messages)
}

fn gemini_messages(path: &Path) -> Result<Vec<SessionMessage>> {
    let Some(session) = read_json_file(path)? else {
        return Ok(Vec::new());
    };
    let Some(values) = session.get("messages").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    Ok(values
        .iter()
        .flat_map(|message| {
            let role = message
                .get("type")
                .or_else(|| message.get("role"))
                .and_then(Value::as_str);
            let content = message.get("content");
            role.zip(content).map_or_else(Vec::new, |(role, content)| {
                messages_from_content(
                    SessionMessageKind::from_provider_role(role),
                    string_at(message, "/timestamp"),
                    content,
                )
            })
        })
        .collect())
}

fn opencode_messages(home: &Path, session_id: &str) -> Result<Vec<SessionMessage>> {
    let storage = home.join(".local/share/opencode/storage");
    let mut messages = Vec::new();
    for message_path in files_with_extension(&storage.join("message").join(session_id), "json")? {
        let Some(message) = read_json_file(&message_path)? else {
            continue;
        };
        let Some(role) = message.get("role").and_then(Value::as_str) else {
            continue;
        };
        let Some(message_id) = string_at(&message, "/id").or_else(|| file_stem(&message_path))
        else {
            continue;
        };
        let created = message.pointer("/time/created").and_then(Value::as_u64);
        let timestamp = message
            .pointer("/time/created")
            .map(Value::to_string)
            .or_else(|| string_at(&message, "/timestamp"));
        let parent_kind = SessionMessageKind::from_provider_role(role);
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
                &part,
            ));
        }
        if parsed.is_empty()
            && let Some(content) = message.get("content")
        {
            parsed.extend(messages_from_content(
                parent_kind,
                timestamp.clone(),
                content,
            ));
        }
        for (part_index, parsed) in parsed.into_iter().enumerate() {
            messages.push((
                created.unwrap_or_default(),
                message_path.clone(),
                part_index,
                parsed,
            ));
        }
    }
    messages.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    Ok(messages
        .into_iter()
        .map(|(_, _, _, message)| message)
        .collect())
}

fn messages_from_content(
    parent_kind: SessionMessageKind,
    timestamp: Option<String>,
    content: &Value,
) -> Vec<SessionMessage> {
    if let Value::Array(parts) = content {
        return parts
            .iter()
            .flat_map(|part| messages_from_content(parent_kind, timestamp.clone(), part))
            .collect();
    }

    let kind =
        content
            .get("type")
            .and_then(Value::as_str)
            .map_or(parent_kind, |value| match value {
                "text" | "input_text" | "output_text" => parent_kind,
                "tool_use" | "tool_call" | "function_call" => SessionMessageKind::ToolCall,
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
                content,
            }]
        })
        .unwrap_or_default()
}

fn opencode_part_messages(
    parent_kind: SessionMessageKind,
    timestamp: Option<String>,
    part: &Value,
) -> Vec<SessionMessage> {
    if part.get("type").and_then(Value::as_str) != Some("tool") {
        return messages_from_content(parent_kind, timestamp, part);
    }

    let mut messages = Vec::new();
    for (pointer, kind) in [
        ("/state/input", SessionMessageKind::ToolCall),
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
        let content = Value::Object(object);
        if let Some(content) = content_text_full(&content) {
            messages.push(SessionMessage {
                kind,
                timestamp: timestamp.clone(),
                content,
            });
        }
    }
    if messages.is_empty()
        && let Some(content) = content_text_full(part)
    {
        messages.push(SessionMessage {
            kind: SessionMessageKind::ToolCall,
            timestamp,
            content,
        });
    }
    messages
}

fn claude_usage(path: &Path) -> Result<(Option<u64>, Option<f64>)> {
    let mut tokens = 0_u64;
    let mut has_usage = false;
    let skipped = visit_bounded_lines(path, |line| {
        let Ok(record) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        if let Some(usage) = record.pointer("/message/usage") {
            has_usage = true;
            for field in [
                "input_tokens",
                "output_tokens",
                "cache_read_input_tokens",
                "cache_creation_input_tokens",
            ] {
                tokens = tokens
                    .saturating_add(usage.get(field).and_then(Value::as_u64).unwrap_or_default());
            }
        }
    })?;
    Ok(((has_usage && !skipped).then_some(tokens), None))
}

fn gemini_usage(path: &Path) -> Result<(Option<u64>, Option<f64>)> {
    let tokens = read_json_file(path)?.and_then(|value| {
        value
            .get("messages")
            .and_then(Value::as_array)
            .and_then(|messages| {
                let totals: Vec<u64> = messages
                    .iter()
                    .filter_map(|message| message.pointer("/tokens/total").and_then(Value::as_u64))
                    .collect();
                (!totals.is_empty()).then(|| totals.into_iter().sum())
            })
    });
    Ok((tokens, None))
}

fn opencode_usage(home: &Path, id: &str) -> Result<(Option<u64>, Option<f64>)> {
    let root = home.join(".local/share/opencode/storage/message").join(id);
    let mut tokens = 0_u64;
    let mut has_tokens = false;
    let mut cost = 0.0_f64;
    let mut has_cost = false;
    for message_path in files_with_extension(&root, "json")? {
        let Some(message) = read_json_file(&message_path)? else {
            continue;
        };
        if let Some(usage) = message.get("tokens") {
            has_tokens = true;
            for pointer in [
                "/input",
                "/output",
                "/reasoning",
                "/cache/read",
                "/cache/write",
            ] {
                tokens = tokens.saturating_add(
                    usage
                        .pointer(pointer)
                        .and_then(Value::as_u64)
                        .unwrap_or_default(),
                );
            }
        }
        if let Some(value) = message.get("cost").and_then(Value::as_f64) {
            has_cost = true;
            cost += value;
        }
    }
    Ok((has_tokens.then_some(tokens), has_cost.then_some(cost)))
}

fn pi_usage(path: &Path) -> Result<(Option<u64>, Option<f64>)> {
    let mut tokens = 0_u64;
    let mut has_tokens = false;
    let mut cost = 0.0_f64;
    let mut has_cost = false;
    let skipped = visit_bounded_lines(path, |line| {
        let Ok(record) = serde_json::from_slice::<Value>(line) else {
            return;
        };
        if let Some(usage) = record.pointer("/message/usage") {
            if let Some(value) = usage.get("totalTokens").and_then(Value::as_u64) {
                has_tokens = true;
                tokens = tokens.saturating_add(value);
            }
            if let Some(value) = usage.pointer("/cost/total").and_then(Value::as_f64) {
                has_cost = true;
                cost += value;
            }
        }
    })?;
    Ok((
        (has_tokens && !skipped).then_some(tokens),
        (has_cost && !skipped).then_some(cost),
    ))
}

pub fn tail_records(path: &Path, limit: usize) -> Result<Vec<String>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut records = VecDeque::with_capacity(limit.min(1_024));
    visit_bounded_lines(path, |line| {
        if records.len() == limit {
            records.pop_front();
        }
        records.push_back(String::from_utf8_lossy(line).into_owned());
    })?;
    Ok(records.into_iter().collect())
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
mod tests {
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    use tempfile::tempdir;

    use super::{SessionCatalog, SessionMessageKind};
    use crate::AgentKind;

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

        let catalog = SessionCatalog::scan(home).expect("scan sessions");
        assert_eq!(catalog.sessions().len(), 6);
        assert_session(
            &catalog,
            &AgentKind::Codex,
            "codex-id",
            "Fix Codex rendering",
            120,
            None,
        );
        assert_session(
            &catalog,
            &AgentKind::ClaudeCode,
            "claude-id",
            "Fix Claude rendering",
            100,
            None,
        );
        assert_session(
            &catalog,
            &AgentKind::GeminiCli,
            "gemini-id",
            "Fix Gemini rendering",
            55,
            None,
        );
        assert_session(
            &catalog,
            &AgentKind::OpenCode,
            "opencode-id",
            "Fix OpenCode rendering",
            65,
            Some(0.25),
        );
        assert_session(
            &catalog,
            &AgentKind::Pi,
            "pi-id",
            "Fix Pi rendering",
            77,
            Some(0.5),
        );
        assert_session(
            &catalog,
            &AgentKind::OhMyPi,
            "omp-id",
            "Fix OMP rendering",
            88,
            Some(0.75),
        );
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
                 {{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"first question\"}}]}}}}\n\
                 {{\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{{\"type\":\"output_text\",\"text\":{long_reply:?}}}]}}}}\n"
            ),
        );
        let catalog = SessionCatalog::scan(temp.path()).expect("scan sessions");
        let session = catalog
            .resolve(Some("codex"), "codex-detail")
            .expect("fixture session");

        let detail = catalog.detail(session).expect("load complete detail");

        assert_eq!(detail.messages.len(), 2);
        assert_eq!(detail.messages[0].kind, SessionMessageKind::User);
        assert_eq!(detail.messages[0].content, "first question");
        assert_eq!(detail.messages[1].kind, SessionMessageKind::Assistant);
        assert_eq!(detail.messages[1].content, long_reply);
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
                SessionMessageKind::ToolResult,
                SessionMessageKind::Assistant,
                SessionMessageKind::System,
                SessionMessageKind::Error,
            ]
        );
        assert!(detail.messages[1].content.contains("read_file"));
        assert!(detail.messages[2].content.contains("file contents"));
        assert!(detail.messages[4].content.contains("meta value"));
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
                "{\"type\":\"assistant\",\"sessionId\":\"claude-detail\",\"timestamp\":\"2026-01-02T03:04:06Z\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"first answer\"}]}}\n",
                "{\"type\":\"user\",\"sessionId\":\"claude-detail\",\"timestamp\":\"2026-01-02T03:04:07Z\",\"message\":{\"role\":\"user\",\"content\":\"second question\"}}\n"
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
                (SessionMessageKind::User, "second question")
            ]
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
                     {{\"type\":\"message\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"before\"}},{{\"type\":\"tool_use\",\"name\":\"read\",\"input\":{{\"path\":\"README.md\"}}}},{{\"type\":\"tool_result\",\"content\":\"file contents\"}},{{\"type\":\"thinking\",\"thinking\":\"private note\"}},{{\"type\":\"error\",\"message\":\"tool failed\"}},{{\"type\":\"text\",\"text\":\"after\"}}]}}}}\n"
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
                    SessionMessageKind::Assistant,
                ],
                "provider {provider}"
            );
            assert!(detail.messages[1].content.contains("README.md"));
            assert!(detail.messages[2].content.contains("file contents"));
            assert!(detail.messages[3].content.contains("private note"));
            assert!(detail.messages[4].content.contains("tool failed"));
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
            r#"{"sessionId":"gemini-detail","messages":[{"type":"user","content":"first question"},{"type":"gemini","content":"first answer"},{"type":"user","content":"second question"}]}"#,
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
                (SessionMessageKind::User, "second question")
            ]
        );
    }

    #[test]
    fn classifies_structured_gemini_parts_without_collapsing_them() {
        let temp = tempdir().expect("temp home");
        let session_path = temp
            .path()
            .join(".gemini/tmp/project/chats/gemini-structured.json");
        write(
            &session_path,
            r#"{"sessionId":"gemini-structured","messages":[{"type":"gemini","content":[{"type":"text","text":"before"},{"type":"function_call","name":"read","args":{"path":"README.md"}},{"type":"function_call_output","output":"file contents"},{"type":"mystery","value":"meta"},{"type":"text","text":"after"}]}]}"#,
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
                SessionMessageKind::Assistant,
            ]
        );
        assert!(detail.messages[1].content.contains("README.md"));
        assert!(detail.messages[2].content.contains("file contents"));
        assert!(detail.messages[3].content.contains("meta"));
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
                     {{\"type\":\"message\",\"message\":{{\"role\":\"assistant\",\"content\":\"answer for {session_id}\"}}}}\n"
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
        }
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
            r#"{"id":"message-2","role":"assistant","time":{"created":3}}"#,
        );
        write(
            &storage.join("part/message-2/part-2.json"),
            r#"{"type":"text","text":"second answer"}"#,
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
            r#"{"type":"tool","tool":"read","state":{"input":{"path":"README.md"},"output":"file contents"}}"#,
        );
        let catalog = SessionCatalog::scan(temp.path()).expect("scan sessions");
        let session = catalog
            .resolve(Some("opencode"), "opencode-detail")
            .expect("fixture session");

        let detail = catalog.detail(session).expect("load complete detail");

        assert_eq!(detail.messages.len(), 4);
        assert_eq!(detail.messages[0].kind, SessionMessageKind::User);
        assert_eq!(detail.messages[0].content, "first question");
        assert_eq!(detail.messages[1].kind, SessionMessageKind::ToolCall);
        assert!(detail.messages[1].content.contains("README.md"));
        assert_eq!(detail.messages[2].kind, SessionMessageKind::ToolResult);
        assert!(detail.messages[2].content.contains("file contents"));
        assert_eq!(detail.messages[3].kind, SessionMessageKind::Assistant);
        assert_eq!(detail.messages[3].content, "second answer");
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
    fn matches_a_recent_session_when_a_gui_agent_reports_the_root_directory() {
        let temp = tempdir().expect("temp home");
        let session_path = temp.path().join(".codex/sessions/2026/01/02/codex.jsonl");
        write(
            &session_path,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"codex-id\",\"cwd\":\"/work/project\"}}\n",
        );
        let catalog = SessionCatalog::scan(temp.path()).expect("scan sessions");
        let updated_at = catalog.sessions()[0].updated_at;

        let matched = catalog
            .latest_for_process(
                &AgentKind::Codex,
                Some(std::path::Path::new("/")),
                updated_at,
            )
            .expect("recent Codex session");

        assert_eq!(matched.id, "codex-id");
        assert_eq!(
            matched.project.as_deref(),
            Some(std::path::Path::new("/work/project"))
        );
        assert!(
            catalog
                .latest_for_process(
                    &AgentKind::Codex,
                    Some(std::path::Path::new("/work/other-project")),
                    updated_at,
                )
                .is_none(),
            "a meaningful cwd must never fall back to another project's session"
        );
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
        let session = catalog.with_usage(session).expect("load session usage");
        assert_eq!(session.id, id);
        assert_eq!(session.title.as_deref(), Some(title));
        assert_eq!(session.tokens, Some(tokens));
        assert_eq!(session.cost_usd, cost_usd);
    }

    fn write(path: &std::path::Path, contents: &str) {
        fs::create_dir_all(path.parent().expect("parent")).expect("create fixture directory");
        fs::write(path, contents).expect("write fixture");
    }
}
