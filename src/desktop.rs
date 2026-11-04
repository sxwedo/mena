//! Native desktop interface over the same catalogs used by the CLI and TUI.
//!
//! Calls are synchronous so the Swift host can own scheduling and cancellation.
//! The UI runs them off its main actor and discards obsolete selection results.

mod launch;
mod model;
mod resources;

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, ensure};

use crate::continuation::{MESSAGE_FORK_PROVIDERS, PreparedContinuation, prepare_message_fork};
use crate::process::{AgentKind, discover_live_agents, find_executable};
use crate::session::{AgentSession, SessionCatalog, SessionDetail, native_resume_command};
use crate::settings::Settings;
pub use model::*;

#[derive(Debug, uniffi::Error)]
pub enum DesktopError {
    Failed { message: String },
}

impl fmt::Display for DesktopError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Failed { message } => formatter.write_str(message),
        }
    }
}

impl std::error::Error for DesktopError {}

impl From<anyhow::Error> for DesktopError {
    fn from(error: anyhow::Error) -> Self {
        Self::Failed {
            message: format!("{error:#}"),
        }
    }
}

/// Owns a context file until its terminal process exits. Dropping the last
/// reference removes the private handoff; callers must retain it during launch.
#[derive(uniffi::Object)]
pub struct DesktopPreparedLaunch {
    data: DesktopLaunch,
    _handoff: Option<PreparedContinuation>,
}

#[uniffi::export]
impl DesktopPreparedLaunch {
    #[must_use]
    pub fn launch(&self) -> DesktopLaunch {
        self.data.clone()
    }
}

#[derive(uniffi::Object)]
pub struct DesktopClient {
    home: PathBuf,
    settings: Settings,
    catalog: Mutex<Option<Arc<SessionCatalog>>>,
    search_generation: AtomicU64,
}

#[uniffi::export]
#[allow(clippy::needless_pass_by_value)] // UniFFI owns strings and optional strings across the FFI seam.
impl DesktopClient {
    /// Create a catalog client. Construct with the actual user home in the app;
    /// tests can provide an isolated fixture home without changing HOME.
    ///
    /// # Errors
    /// Returns an error for an invalid home directory or unreadable configuration.
    #[uniffi::constructor]
    pub fn new(home_path: String) -> Result<Arc<Self>, DesktopError> {
        let home = PathBuf::from(home_path);
        validate_directory(&home)?;
        Ok(Arc::new(Self {
            home,
            settings: Settings::load()?,
            catalog: Mutex::new(None),
            search_generation: AtomicU64::new(0),
        }))
    }

    /// Refresh metadata and exact process associations. Subagent visibility is
    /// a UI preference; the complete catalog always stays available here.
    ///
    /// # Errors
    /// Returns discovery, configuration, or native-association failures.
    pub fn refresh(&self) -> Result<DesktopSnapshot, DesktopError> {
        let catalog = Arc::new(SessionCatalog::scan_provider(
            &self.home, None, false, false,
        )?);
        let agents = discover_live_agents(&self.settings.agent.custom)?;
        let processes = agents
            .iter()
            .map(|agent| {
                Ok(DesktopProcess {
                    pid: agent.process.pid,
                    provider: agent.kind.slug().to_owned(),
                    project: agent
                        .process
                        .cwd
                        .as_ref()
                        .map(|path| path.display().to_string()),
                    runtime_seconds: agent.process.run_time,
                    memory_bytes: agent.process.memory_bytes,
                    status: agent.process.status.clone(),
                    session_target: catalog.active_target(agent)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let active: std::collections::BTreeSet<_> = processes
            .iter()
            .filter_map(|process| process.session_target.as_deref())
            .collect();
        let sessions = catalog
            .sessions()
            .iter()
            .map(|session| {
                DesktopSession::from_session(session, active.contains(session.target().as_str()))
            })
            .collect();
        let snapshot = DesktopSnapshot {
            sessions,
            processes,
            warnings: catalog.warnings().to_vec(),
            agents: AgentKind::all_kinds(&self.settings.agent.custom)
                .iter()
                .map(|kind| DesktopAgent {
                    provider: kind.slug().to_owned(),
                    name: kind.to_string(),
                    executable: kind
                        .executable_name(&self.settings.agent.custom)
                        .and_then(find_executable)
                        .map(|path| path.display().to_string()),
                    homepage: kind.homepage_url().to_owned(),
                    custom: matches!(kind, AgentKind::Custom(_)),
                })
                .collect(),
            launch_providers: AgentKind::all_kinds(&self.settings.agent.custom)
                .iter()
                .filter(|kind| {
                    kind.executable_name(&self.settings.agent.custom)
                        .and_then(find_executable)
                        .is_some()
                })
                .map(|kind| kind.slug().to_owned())
                .collect(),
            context_providers: MESSAGE_FORK_PROVIDERS
                .iter()
                .filter(|kind| {
                    kind.executable_name(&self.settings.agent.custom)
                        .and_then(find_executable)
                        .is_some()
                })
                .map(|kind| kind.slug().to_owned())
                .collect(),
        };
        *self
            .catalog
            .lock()
            .map_err(|_| anyhow::anyhow!("会话目录状态不可用。"))? = Some(catalog);
        Ok(snapshot)
    }

    /// Load bounded, normalized messages for an exact saved-session target.
    ///
    /// # Errors
    /// Returns an error for a missing target, invalid storage, or a failed read.
    pub fn session_detail(&self, target: String) -> Result<DesktopSessionDetail, DesktopError> {
        let catalog = self.catalog()?;
        let detail = catalog.detail(resolve(&catalog, &target)?)?;
        Ok(DesktopSessionDetail::from_detail(&detail))
    }

    /// Bounded, cancellable transcript search for the current UI selection.
    #[must_use]
    pub fn begin_search(&self) -> u64 {
        self.search_generation.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Search normalized messages under the ticket returned by `begin_search`.
    ///
    /// # Errors
    /// Returns an error when a requested session cannot be resolved or read.
    pub fn search_sessions(
        &self,
        query: String,
        targets: Vec<String>,
        generation: u64,
    ) -> Result<Vec<DesktopSearchHit>, DesktopError> {
        let query = query.trim().to_ascii_lowercase();
        if query.is_empty() || query.len() > 2048 {
            return Ok(Vec::new());
        }
        let catalog = self.catalog()?;
        let mut hits = Vec::new();
        for target in targets {
            if self.search_generation.load(Ordering::Relaxed) != generation {
                break;
            }
            let session = resolve(&catalog, &target)?;
            // Search normalized messages, not the backing file. A provider may
            // keep unrelated conversations in one SQLite database.
            let detail = catalog.detail(session)?;
            for message in detail.messages {
                if self.search_generation.load(Ordering::Relaxed) != generation {
                    return Ok(hits);
                }
                if let Some(at) = message.content.to_ascii_lowercase().find(&query) {
                    let start = message
                        .content
                        .char_indices()
                        .map(|(index, _)| index)
                        .take_while(|index| *index <= at.saturating_sub(80))
                        .last()
                        .unwrap_or(0);
                    let snippet = message.content[start..].chars().take(240).collect();
                    hits.push(DesktopSearchHit { target, snippet });
                    break;
                }
            }
        }
        Ok(hits)
    }

    pub fn cancel_search(&self) {
        self.search_generation.fetch_add(1, Ordering::Relaxed);
    }

    /// Save a mena-owned title overlay, leaving the transcript untouched.
    ///
    /// # Errors
    /// Returns validation, target-resolution, or overlay-write failures.
    pub fn rename_session(&self, target: String, title: String) -> Result<(), DesktopError> {
        let catalog = self.fresh_catalog()?;
        catalog.set_title(resolve(&catalog, &target)?, &title)?;
        Ok(())
    }

    /// Recheck exact live evidence and native containment immediately before deletion.
    ///
    /// # Errors
    /// Refuses unconfirmed protection, invalid targets, and any confirmation other than `y`.
    pub fn delete_session(
        &self,
        target: String,
        permanent: bool,
        confirmation: String,
    ) -> Result<(), DesktopError> {
        if confirmation != "y" {
            return Err(anyhow::anyhow!("请输入小写 y 确认删除。").into());
        }
        let live = discover_live_agents(&self.settings.agent.custom)?;
        self.delete_with_live_agents(&target, permanent, &live)?;
        Ok(())
    }

    /// List recoverable sessions from mena's own trash.
    ///
    /// # Errors
    /// Returns errors reading the private trash directory.
    pub fn trash_sessions(&self) -> Result<Vec<DesktopTrashSession>, DesktopError> {
        Ok(crate::session::list_trashed_sessions(&self.home)?
            .into_iter()
            .map(|entry| DesktopTrashSession {
                key: entry.key,
                target: entry.target,
                provider: entry.provider,
                title: entry.title,
                deleted_at: entry.deleted_at,
            })
            .collect())
    }

    /// Restore a selected trash entry, refusing collisions and containment escapes.
    ///
    /// # Errors
    /// Returns errors for missing entries, unsafe paths, or existing destinations.
    pub fn restore_session(&self, key: String) -> Result<String, DesktopError> {
        let entries = crate::session::list_trashed_sessions(&self.home)?;
        if !entries.iter().any(|entry| entry.key == key) {
            return Err(anyhow::anyhow!("垃圾站记录已不存在，请刷新。").into());
        }
        Ok(crate::session::restore_session(&self.home, &key)?)
    }

    /// Export fresh normalized history using the same private, non-clobbering
    /// Markdown exporter as the CLI. The UI chooses the destination directory.
    ///
    /// # Errors
    /// Returns target-resolution, bounded-read, or destination-write failures.
    pub fn export_session(
        &self,
        target: String,
        directory: String,
        all_messages: bool,
    ) -> Result<String, DesktopError> {
        let directory = Path::new(&directory);
        validate_directory(directory)?;
        let catalog = self.fresh_catalog()?;
        let detail = catalog.detail(resolve(&catalog, &target)?)?;
        let scope = if all_messages {
            crate::session::DetailScope::All
        } else {
            crate::session::DetailScope::Conversation
        };
        let path = crate::export::export_session_detail(&detail, directory, scope)?;
        Ok(path.display().to_string())
    }

    /// Preview the native command even when its executable or project is unavailable.
    ///
    /// # Errors
    /// Returns target-resolution or unsupported native-resume failures.
    pub fn resume_preview(&self, target: String) -> Result<DesktopResumePreview, DesktopError> {
        let catalog = self.fresh_catalog()?;
        let session = resolve(&catalog, &target)?;
        let command = native_resume_command(&session.kind, &session.id)?;
        let executable = find_executable(&command.program);
        let directory = session
            .project
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        let unavailable_reason = if executable.is_none() {
            Some(format!(
                "尚未安装 {}；你仍可以复制原生恢复命令。",
                session.kind
            ))
        } else if session.project.as_ref().is_none_or(|path| !path.is_dir()) {
            Some("原项目目录已不存在；你仍可以复制恢复命令，在合适的目录中执行。".to_owned())
        } else {
            None
        };
        Ok(DesktopResumePreview {
            launch: DesktopLaunch {
                executable: executable.map_or(command.program, |path| path.display().to_string()),
                arguments: command.args,
                directory,
                title: session.title.clone().unwrap_or_else(|| target.clone()),
                target: Some(target),
            },
            unavailable_reason,
        })
    }

    /// Prepare program and argv without starting a process.
    ///
    /// # Errors
    /// Returns an error for missing sessions, executables, or project directories.
    pub fn prepare_resume(
        &self,
        target: String,
    ) -> Result<Arc<DesktopPreparedLaunch>, DesktopError> {
        let catalog = self.fresh_catalog()?;
        let session = resolve(&catalog, &target)?;
        let command = native_resume_command(&session.kind, &session.id)?;
        Ok(Arc::new(DesktopPreparedLaunch::prepare(
            command,
            session,
            session.title.clone().unwrap_or_else(|| target.clone()),
            Some(target),
            None,
        )?))
    }

    /// Prepare a fresh native Agent in the user-selected project directory.
    ///
    /// # Errors
    /// Returns an error for unknown providers, missing executables, or invalid directories.
    pub fn prepare_agent(
        &self,
        provider: String,
        directory: String,
    ) -> Result<Arc<DesktopPreparedLaunch>, DesktopError> {
        let kind = AgentKind::all_kinds(&self.settings.agent.custom)
            .into_iter()
            .find(|kind| kind.slug() == provider)
            .context("未知 Agent，请刷新后重新选择。")?;
        let directory = Path::new(&directory);
        validate_directory(directory)?;
        let command = crate::controller::fresh_launch_spec(&kind, &self.settings.agent.custom)?;
        Ok(Arc::new(DesktopPreparedLaunch::in_directory(
            command,
            directory,
            format!("{kind} · 新对话"),
            None,
            None,
        )?))
    }

    /// Re-read and validate the entire displayed history before either fork
    /// route. Neither a stale row index nor a changed prefix can be accepted.
    ///
    /// # Errors
    /// Returns an error for changed history, unavailable providers, failed native
    /// fork requests, invalid titles, or private context-file creation failures.
    pub fn prepare_fork(
        &self,
        request: DesktopForkRequest,
    ) -> Result<Arc<DesktopPreparedLaunch>, DesktopError> {
        let DesktopForkRequest {
            target,
            revision,
            message_id,
            method,
            provider,
            title,
        } = request;
        let catalog = self.fresh_catalog()?;
        let session = resolve(&catalog, &target)?;
        let detail = catalog.detail(session)?;
        let index = checked_message(&detail, &revision, &message_id)?;
        if !detail.messages[index].kind.can_fork() {
            return Err(anyhow::anyhow!("只能从 USER 或 Agent 消息分叉。").into());
        }
        // Resolve launch prerequisites before creating a native fork or a file.
        let directory = session.project.as_deref().context("会话没有项目目录。")?;
        validate_directory(directory)?;
        let prepared = match method {
            DesktopForkMethod::Native => {
                let fork = catalog.fork_native(&detail, index, Some(&title))?;
                let fork_target = format!("{}:{}", session.kind.slug(), fork.session_id);
                DesktopPreparedLaunch::prepare(
                    fork.command,
                    session,
                    title,
                    Some(fork_target),
                    None,
                )?
            }
            DesktopForkMethod::Context => {
                let kind = context_provider(&provider)?;
                let executable = kind
                    .executable_name(&self.settings.agent.custom)
                    .and_then(find_executable)
                    .context("目标 Agent 尚未安装。")?;
                let handoff = prepare_message_fork(&detail, index, &kind)?;
                let mut command = handoff.command().clone();
                command.program = executable.display().to_string();
                DesktopPreparedLaunch::prepare(command, session, title, None, Some(handoff))?
            }
        };
        Ok(Arc::new(prepared))
    }

    /// Statically discover resources in user scope and an optional project.
    ///
    /// # Errors
    /// Returns directory-validation or catalog-discovery failures.
    pub fn resources(
        &self,
        kind: DesktopResourceKind,
        workspace: Option<String>,
    ) -> Result<Vec<DesktopResource>, DesktopError> {
        Ok(resources::list(
            &self.home,
            workspace_path(workspace.as_deref())?,
            kind,
        )?)
    }

    /// Revalidate and read one resource through its owning catalog.
    ///
    /// # Errors
    /// Returns an error for missing, escaped, unreadable, or oversized resources.
    pub fn resource_detail(
        &self,
        kind: DesktopResourceKind,
        id: String,
        workspace: Option<String>,
    ) -> Result<DesktopResourceDetail, DesktopError> {
        Ok(resources::detail(
            &self.home,
            workspace_path(workspace.as_deref())?,
            kind,
            &id,
        )?)
    }

    /// This is the only desktop entry point that can initiate an MCP probe.
    ///
    /// # Errors
    /// Returns an error for invalid workspace paths or missing registrations.
    /// Bounded connection/protocol failures are returned as probe metadata.
    pub fn probe_mcp(
        &self,
        id: String,
        workspace: Option<String>,
    ) -> Result<DesktopResourceDetail, DesktopError> {
        Ok(resources::probe(
            &self.home,
            workspace_path(workspace.as_deref())?,
            &id,
        )?)
    }
}

impl DesktopClient {
    fn delete_with_live_agents(
        &self,
        target: &str,
        permanent: bool,
        live: &[crate::process::LiveAgent],
    ) -> Result<()> {
        let catalog = self.fresh_catalog()?;
        ensure!(
            !catalog.protection(live)?.protected_targets.contains(target),
            "会话正在运行，或当前运行关联尚未确认，不能删除。"
        );
        let session = resolve(&catalog, target)?;
        if permanent {
            catalog.delete_session_permanently(session)?;
        } else {
            catalog.delete_session(session)?;
        }
        Ok(())
    }

    fn catalog(&self) -> Result<Arc<SessionCatalog>> {
        let cached = self
            .catalog
            .lock()
            .map_err(|_| anyhow::anyhow!("会话目录状态不可用。"))?
            .clone();
        if let Some(catalog) = cached {
            return Ok(catalog);
        }
        Ok(Arc::new(self.fresh_catalog()?))
    }

    fn fresh_catalog(&self) -> Result<SessionCatalog> {
        SessionCatalog::scan_provider(&self.home, None, false, false)
    }
}

fn resolve<'a>(catalog: &'a SessionCatalog, target: &str) -> Result<&'a AgentSession> {
    let (provider, id) = target
        .split_once(':')
        .context("需要完整的 provider:session-id 会话标识。")?;
    let session = catalog.resolve(Some(provider), id)?;
    ensure!(
        session.target() == target,
        "操作需要完整的会话标识，不能使用前缀。"
    );
    Ok(session)
}

fn checked_message(detail: &SessionDetail, revision: &str, id: &str) -> Result<usize> {
    ensure!(
        model::detail_revision(detail) == revision,
        "会话内容已更新。请刷新详情，再选择分叉起点。"
    );
    detail
        .messages
        .iter()
        .enumerate()
        .find_map(|(index, message)| (model::message_key(index, message) == id).then_some(index))
        .context("选中的消息已不存在。请刷新详情。")
}

fn context_provider(provider: &str) -> Result<AgentKind> {
    let kind = AgentKind::from_slug(provider).context("未知目标 Agent。")?;
    ensure!(
        MESSAGE_FORK_PROVIDERS.contains(&kind),
        "目标 Agent 不支持上下文新建。"
    );
    Ok(kind)
}

fn workspace_path(workspace: Option<&str>) -> Result<Option<&Path>> {
    workspace
        .map(|path| {
            let path = Path::new(path);
            ensure!(
                path.is_absolute() && path.is_dir(),
                "项目目录必须是存在的绝对路径。"
            );
            Ok(path)
        })
        .transpose()
}

fn validate_directory(path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute() && path.is_dir(),
        "目录必须是存在的绝对路径：{}",
        path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests;
