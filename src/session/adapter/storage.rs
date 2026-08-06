//! Provider-owned storage layouts, metadata discovery, and index cleanup.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags, params};
use serde_json::Value;

use super::super::{
    AgentSession, content_preview, file_stem, files_with_extension, normalize_preview,
    paths_equivalent, read_json_file, remove_jsonl_records, session, string_at,
    validate_storage_identifier, visit_bounded_lines_limit,
};
use crate::{AgentKind, ProcessSnapshot};

const PROCESS_START_TOLERANCE_SECONDS: u64 = 5;

pub(super) fn scan_codex(home: &Path, sessions: &mut Vec<AgentSession>) -> Result<()> {
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

pub(super) fn scan_claude(home: &Path, sessions: &mut Vec<AgentSession>) -> Result<()> {
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

pub(super) fn scan_gemini(home: &Path, sessions: &mut Vec<AgentSession>) -> Result<()> {
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

pub(super) fn scan_opencode(home: &Path, sessions: &mut Vec<AgentSession>) -> Result<()> {
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

pub(super) fn scan_pi(home: &Path, sessions: &mut Vec<AgentSession>) -> Result<()> {
    scan_pi_sessions(&home.join(".pi/agent/sessions"), &AgentKind::Pi, sessions)
}

pub(super) fn scan_oh_my_pi(home: &Path, sessions: &mut Vec<AgentSession>) -> Result<()> {
    scan_pi_sessions(
        &home.join(".omp/agent/sessions"),
        &AgentKind::OhMyPi,
        sessions,
    )
}

pub(super) fn runtime_claude_session_ids(
    home: &Path,
    process: &ProcessSnapshot,
) -> Result<Vec<String>> {
    let path = home
        .join(".claude/sessions")
        .join(format!("{}.json", process.pid));
    if !path.exists() {
        return Ok(Vec::new());
    }
    let Some(record) = read_json_file(&path)? else {
        return Ok(Vec::new());
    };
    if record.get("pid").and_then(Value::as_u64) != Some(u64::from(process.pid)) {
        return Ok(Vec::new());
    }
    let Some(started_at_ms) = record.get("startedAt").and_then(Value::as_u64) else {
        return Ok(Vec::new());
    };
    if (started_at_ms / 1_000).abs_diff(process.started_at) > PROCESS_START_TOLERANCE_SECONDS {
        return Ok(Vec::new());
    }
    let Some(runtime_cwd) = string_at(&record, "/cwd").map(PathBuf::from) else {
        return Ok(Vec::new());
    };
    if !process
        .cwd
        .as_deref()
        .is_some_and(|cwd| paths_equivalent(cwd, &runtime_cwd))
    {
        return Ok(Vec::new());
    }
    Ok(string_at(&record, "/sessionId").into_iter().collect())
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

pub(super) fn collect_codex_artifacts(
    home: &Path,
    session: &AgentSession,
    files: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    collect_files_with_prefix(
        &home.join(".codex/shell_snapshots"),
        &format!("{}.", session.id),
        files,
    )
}

pub(super) fn collect_claude_artifacts(
    home: &Path,
    session: &AgentSession,
    files: &mut BTreeSet<PathBuf>,
    directories: &mut BTreeSet<PathBuf>,
) -> Result<()> {
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
    )
}

pub(super) fn collect_opencode_artifacts(
    home: &Path,
    session: &AgentSession,
    files: &mut BTreeSet<PathBuf>,
    directories: &mut BTreeSet<PathBuf>,
) -> Result<()> {
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
    Ok(())
}

pub(super) fn delete_codex_index_records(home: &Path, session_id: &str) -> Result<usize> {
    Ok(delete_codex_database_rows(home, session_id)?
        + remove_jsonl_records(&home.join(".codex/session_index.jsonl"), "/id", session_id)?)
}

pub(super) fn delete_claude_index_records(home: &Path, session_id: &str) -> Result<usize> {
    remove_jsonl_records(
        &home.join(".claude/history.jsonl"),
        "/sessionId",
        session_id,
    )
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
            .busy_timeout(Duration::from_secs(2))
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
    sqlite_table_exists_conn(transaction, table)
}
pub(super) fn cursor_global_storage_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        home.join("Library/Application Support/Cursor/User/globalStorage"),
        home.join(".config/Cursor/User/globalStorage"),
        home.join("AppData/Roaming/Cursor/User/globalStorage"),
    ]
}

#[allow(
    clippy::too_many_lines,
    clippy::unnecessary_wraps,
    clippy::collapsible_if
)]
pub(super) fn scan_cursor(home: &Path, sessions: &mut Vec<AgentSession>) -> Result<()> {
    for global_dir in cursor_global_storage_dirs(home) {
        let db_path = global_dir.join("state.vscdb");
        if !db_path.is_file() {
            continue;
        }
        let Ok(connection) = Connection::open_with_flags(
            &db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) else {
            continue;
        };

        let mut parsed = false;

        if sqlite_table_exists_conn(&connection, "composerHeaders").unwrap_or(false) {
            if let Ok(mut stmt) = connection.prepare(
                "SELECT composerId, createdAt, lastUpdatedAt, isArchived, value FROM composerHeaders",
            ) {
                let rows = stmt.query_map([], |row| {
                    let id: String = row.get(0)?;
                    let created_at: Option<u64> =
                        row.get::<_, Option<i64>>(1).ok().flatten().and_then(|v| u64::try_from(v).ok());
                    let last_updated_at: Option<u64> =
                        row.get::<_, Option<i64>>(2).ok().flatten().and_then(|v| u64::try_from(v).ok());
                    let is_archived: Option<i64> = row.get(3).ok();
                    let value_str: Option<String> = row.get(4).ok();
                    Ok((id, created_at, last_updated_at, is_archived, value_str))
                });

                if let Ok(rows) = rows {
                    for row in rows.flatten() {
                        let (id, created_at, last_updated_at, _is_archived, value_str) = row;
                        let (mut title, project, created_ms, updated_ms) =
                            parse_composer_json(value_str.as_deref());
                        if title.is_none() {
                            title = load_cursor_first_user_message_preview(&connection, &id)
                                .or_else(|| Some("(empty session)".to_owned()));
                        }
                        let updated_secs = updated_ms
                            .or(last_updated_at)
                            .or(created_ms)
                            .or(created_at)
                            .map(|ms| ms / 1000);

                        if let Ok(session_obj) = session(
                            AgentKind::Cursor,
                            id,
                            title,
                            project,
                            db_path.clone(),
                            None,
                            None,
                            None,
                        ) {
                            let mut sess = session_obj;
                            if let Some(updated) = updated_secs {
                                sess.updated_at = updated;
                            }
                            sessions.push(sess);
                            parsed = true;
                        }
                    }
                }
            }
        }

        if !parsed && sqlite_table_exists_conn(&connection, "ItemTable").unwrap_or(false) {
            if let Ok(mut stmt) = connection.prepare(
                "SELECT CAST(value AS TEXT) FROM ItemTable WHERE key = 'composer.composerHeaders'",
            ) {
                if let Ok(mut rows) = stmt.query([]) {
                    if let Ok(Some(row)) = rows.next() {
                        let value_str: String = row.get(0).unwrap_or_default();
                        if let Ok(value) = serde_json::from_str::<Value>(&value_str) {
                            if let Some(composers) =
                                value.get("allComposers").and_then(Value::as_array)
                            {
                                for comp in composers {
                                    let Some(id) = string_at(comp, "/composerId") else {
                                        continue;
                                    };
                                    let mut title = string_at(comp, "/name")
                                        .or_else(|| string_at(comp, "/text"))
                                        .or_else(|| string_at(comp, "/subtitle"));
                                    if title.is_none() {
                                        title = load_cursor_first_user_message_preview(
                                            &connection,
                                            &id,
                                        )
                                        .or_else(|| Some("(empty session)".to_owned()));
                                    }
                                    let project = comp
                                        .pointer("/workspaceIdentifier/uri/fsPath")
                                        .and_then(Value::as_str)
                                        .or_else(|| {
                                            comp.pointer("/workspaceIdentifier/uri/path")
                                                .and_then(Value::as_str)
                                        })
                                        .map(PathBuf::from);
                                    let updated_secs = comp
                                        .get("lastUpdatedAt")
                                        .or_else(|| comp.get("createdAt"))
                                        .and_then(Value::as_u64)
                                        .map(|ms| ms / 1000);

                                    if let Ok(session_obj) = session(
                                        AgentKind::Cursor,
                                        id,
                                        title,
                                        project,
                                        db_path.clone(),
                                        None,
                                        None,
                                        None,
                                    ) {
                                        let mut sess = session_obj;
                                        if let Some(updated) = updated_secs {
                                            sess.updated_at = updated;
                                        }
                                        sessions.push(sess);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

#[allow(clippy::unnecessary_wraps, clippy::missing_const_for_fn)]
pub(super) fn collect_cursor_artifacts(
    _home: &Path,
    _session: &AgentSession,
    _files: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    Ok(())
}

#[allow(clippy::unnecessary_wraps, clippy::collapsible_if)]
pub(super) fn delete_cursor_index_records(home: &Path, session_id: &str) -> Result<usize> {
    let mut removed = 0_usize;
    for global_dir in cursor_global_storage_dirs(home) {
        let db_path = global_dir.join("state.vscdb");
        if !db_path.is_file() {
            continue;
        }
        let Ok(mut connection) = Connection::open_with_flags(
            &db_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) else {
            continue;
        };
        let Ok(transaction) = connection.transaction() else {
            continue;
        };

        if sqlite_table_exists_conn(&transaction, "composerHeaders").unwrap_or(false) {
            if let Ok(cnt) = transaction.execute(
                "DELETE FROM composerHeaders WHERE composerId = ?1",
                params![session_id],
            ) {
                removed += cnt;
            }
        }

        let composer_key = format!("composerData:{session_id}");
        let bubble_pattern = format!("bubbleId:{session_id}:%");

        if sqlite_table_exists_conn(&transaction, "cursorDiskKV").unwrap_or(false) {
            if let Ok(cnt) = transaction.execute(
                "DELETE FROM cursorDiskKV WHERE key = ?1 OR key LIKE ?2",
                params![composer_key, bubble_pattern],
            ) {
                removed += cnt;
            }
        }

        if sqlite_table_exists_conn(&transaction, "ItemTable").unwrap_or(false) {
            if let Ok(cnt) = transaction.execute(
                "DELETE FROM ItemTable WHERE key = ?1 OR key LIKE ?2",
                params![composer_key, bubble_pattern],
            ) {
                removed += cnt;
            }
        }

        let _ = transaction.commit();
    }
    Ok(removed)
}

fn parse_composer_json(
    json_str: Option<&str>,
) -> (Option<String>, Option<PathBuf>, Option<u64>, Option<u64>) {
    let Some(json_str) = json_str else {
        return (None, None, None, None);
    };
    let Ok(val) = serde_json::from_str::<Value>(json_str) else {
        return (None, None, None, None);
    };
    let title = string_at(&val, "/name")
        .or_else(|| string_at(&val, "/text"))
        .or_else(|| string_at(&val, "/subtitle"));
    let project = val
        .pointer("/workspaceIdentifier/uri/fsPath")
        .and_then(Value::as_str)
        .or_else(|| {
            val.pointer("/workspaceIdentifier/uri/path")
                .and_then(Value::as_str)
        })
        .map(PathBuf::from);
    let created_ms = val.get("createdAt").and_then(Value::as_u64);
    let updated_ms = val.get("lastUpdatedAt").and_then(Value::as_u64);
    (title, project, created_ms, updated_ms)
}

fn sqlite_table_exists_conn(connection: &Connection, table: &str) -> Result<bool> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
            params![table],
            |row| row.get(0),
        )
        .context("failed to inspect a database schema")
}
#[allow(clippy::collapsible_if)]
fn load_cursor_first_user_message_preview(
    connection: &Connection,
    session_id: &str,
) -> Option<String> {
    let key = format!("composerData:{session_id}");
    let mut json_str = None;

    if sqlite_table_exists_conn(connection, "cursorDiskKV").unwrap_or(false) {
        if let Ok(mut stmt) =
            connection.prepare("SELECT CAST(value AS TEXT) FROM cursorDiskKV WHERE key = ?1")
        {
            if let Ok(mut rows) = stmt.query(params![key]) {
                if let Ok(Some(row)) = rows.next() {
                    json_str = row.get::<_, String>(0).ok();
                }
            }
        }
    }

    if json_str.is_none() && sqlite_table_exists_conn(connection, "ItemTable").unwrap_or(false) {
        if let Ok(mut stmt) =
            connection.prepare("SELECT CAST(value AS TEXT) FROM ItemTable WHERE key = ?1")
        {
            if let Ok(mut rows) = stmt.query(params![key]) {
                if let Ok(Some(row)) = rows.next() {
                    json_str = row.get::<_, String>(0).ok();
                }
            }
        }
    }

    if let Some(json_str) = json_str {
        if let Ok(val) = serde_json::from_str::<Value>(&json_str) {
            if let Some(text) = val.get("text").and_then(Value::as_str) {
                if let Some(preview) = normalize_preview(text) {
                    return Some(preview);
                }
            }
            if let Some(conversation) = val.get("conversation").and_then(Value::as_array) {
                for item in conversation {
                    let msg_type = item.get("type").and_then(Value::as_u64).unwrap_or(0);
                    if msg_type == 1 {
                        if let Some(text) = item.get("text").and_then(Value::as_str) {
                            if let Some(preview) = normalize_preview(text) {
                                return Some(preview);
                            }
                        }
                        if let Some(rich) = item.get("richText").and_then(Value::as_str) {
                            if let Ok(rich_val) = serde_json::from_str::<Value>(rich) {
                                let mut extracted = String::new();
                                collect_lexical_text_preview(&rich_val, &mut extracted);
                                if let Some(preview) = normalize_preview(&extracted) {
                                    return Some(preview);
                                }
                            }
                        }
                    }
                }
            }
            if let Some(headers) = val
                .get("fullConversationHeadersOnly")
                .and_then(Value::as_array)
            {
                for head in headers {
                    let is_user = head.get("type").and_then(Value::as_u64) == Some(1);
                    if is_user {
                        if let Some(bubble_id) = string_at(head, "/bubbleId") {
                            let bubble_key = format!("bubbleId:{session_id}:{bubble_id}");
                            if let Some(preview) = load_bubble_text_preview(connection, &bubble_key)
                            {
                                return Some(preview);
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

#[allow(clippy::collapsible_if)]
fn load_bubble_text_preview(connection: &Connection, bubble_key: &str) -> Option<String> {
    let mut bubble_json = None;
    if sqlite_table_exists_conn(connection, "cursorDiskKV").unwrap_or(false) {
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
    if let Some(json_str) = bubble_json {
        if let Ok(val) = serde_json::from_str::<Value>(&json_str) {
            if let Some(text) = val.get("text").and_then(Value::as_str) {
                if let Some(preview) = normalize_preview(text) {
                    return Some(preview);
                }
            }
            if let Some(rich) = val.get("richText").and_then(Value::as_str) {
                if let Ok(rich_val) = serde_json::from_str::<Value>(rich) {
                    let mut extracted = String::new();
                    collect_lexical_text_preview(&rich_val, &mut extracted);
                    if let Some(preview) = normalize_preview(&extracted) {
                        return Some(preview);
                    }
                }
            }
        }
    }
    None
}

fn collect_lexical_text_preview(val: &Value, out: &mut String) {
    if let Some(text) = val.get("text").and_then(Value::as_str) {
        out.push_str(text);
    }
    if let Some(children) = val.get("children").and_then(Value::as_array) {
        for child in children {
            collect_lexical_text_preview(child, out);
        }
    }
}
