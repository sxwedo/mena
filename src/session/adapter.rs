use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::{
    AgentSession, AssociationEvidence, DeletionSummary, SessionDetail, paths_equivalent,
    remove_file_if_present, remove_tree_if_present, validate_deletion_targets,
    validate_storage_identifier,
};
use crate::{AgentKind, ProcessSnapshot};
use anyhow::{Result, bail};

mod detail;
mod storage;

use crate::process::open_file_paths;
use storage::{
    collect_claude_artifacts, collect_codex_artifacts, collect_cursor_artifacts,
    collect_grok_artifacts, collect_opencode_artifacts, cursor_global_storage_dirs,
    delete_claude_index_records, delete_codex_index_records, delete_cursor_index_records,
    grok_sessions_root, runtime_claude_session_ids, runtime_grok_session_ids, scan_claude,
    scan_codex, scan_cursor, scan_gemini, scan_grok, scan_oh_my_pi, scan_opencode, scan_pi,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProcessEvidence {
    selector: ProcessSelector,
    pub(super) source: AssociationEvidence,
}

impl ProcessEvidence {
    pub(super) fn matches(&self, session: &AgentSession) -> bool {
        match &self.selector {
            ProcessSelector::Id(id) => session.id == *id,
            ProcessSelector::Path(path) => {
                paths_equivalent(&session.path, path)
                    || session
                        .related_paths
                        .iter()
                        .any(|related| paths_equivalent(related, path))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProcessSelector {
    Id(String),
    Path(PathBuf),
}

/// A built-in provider adapter selected without allocation or dynamic dispatch.
///
/// The enum is deliberately closed: adding a provider makes every capability
/// match non-exhaustive until discovery, detail, resume, and deletion semantics
/// have been considered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProviderAdapter {
    ClaudeCode,
    Codex,
    Goose,
    GeminiCli,
    OpenCode,
    Pi,
    OhMyPi,
    Cursor,
    Grok,
}

impl ProviderAdapter {
    /// Providers with a native local session catalog, in stable display order.
    pub(super) const SESSION_CATALOG: [Self; 9] = [
        Self::Codex,
        Self::ClaudeCode,
        Self::Goose,
        Self::GeminiCli,
        Self::OpenCode,
        Self::Pi,
        Self::OhMyPi,
        Self::Cursor,
        Self::Grok,
    ];

    pub(super) const fn from_kind(kind: &AgentKind) -> Option<Self> {
        match kind {
            AgentKind::ClaudeCode => Some(Self::ClaudeCode),
            AgentKind::Codex => Some(Self::Codex),
            AgentKind::Goose => Some(Self::Goose),
            AgentKind::GeminiCli => Some(Self::GeminiCli),
            AgentKind::OpenCode => Some(Self::OpenCode),
            AgentKind::Pi => Some(Self::Pi),
            AgentKind::OhMyPi => Some(Self::OhMyPi),
            AgentKind::Cursor => Some(Self::Cursor),
            AgentKind::Grok => Some(Self::Grok),
            AgentKind::Custom(_) => None,
        }
    }

    pub(super) const fn kind(self) -> AgentKind {
        match self {
            Self::ClaudeCode => AgentKind::ClaudeCode,
            Self::Codex => AgentKind::Codex,
            Self::Goose => AgentKind::Goose,
            Self::GeminiCli => AgentKind::GeminiCli,
            Self::OpenCode => AgentKind::OpenCode,
            Self::Pi => AgentKind::Pi,
            Self::OhMyPi => AgentKind::OhMyPi,
            Self::Cursor => AgentKind::Cursor,
            Self::Grok => AgentKind::Grok,
        }
    }

    pub(super) fn matches(self, kind: &AgentKind) -> bool {
        Self::from_kind(kind) == Some(self)
    }

    pub(super) fn process_evidence(
        self,
        home: &Path,
        process: &ProcessSnapshot,
    ) -> Result<Vec<ProcessEvidence>> {
        if self == Self::Grok {
            return grok_process_evidence(home, process, self);
        }
        let (runtime_selectors, runtime_source) = match self {
            Self::Grok => unreachable!("Grok process evidence is handled above"),
            Self::ClaudeCode => (
                runtime_claude_session_ids(home, process)?
                    .into_iter()
                    .map(ProcessSelector::Id)
                    .collect(),
                AssociationEvidence::NativeRuntime,
            ),
            Self::Pi | Self::OhMyPi => {
                let root = match self {
                    Self::Pi => home.join(".pi/agent/sessions"),
                    Self::OhMyPi => home.join(".omp/agent/sessions"),
                    _ => unreachable!("Pi family checked above"),
                };
                let resolved_root = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
                (
                    open_file_paths(process.pid)
                        .into_iter()
                        .filter(|path| path.starts_with(&root) || path.starts_with(&resolved_root))
                        .map(ProcessSelector::Path)
                        .collect(),
                    AssociationEvidence::OpenSessionFile,
                )
            }
            Self::Codex | Self::Goose | Self::GeminiCli | Self::OpenCode | Self::Cursor => {
                (Vec::new(), AssociationEvidence::NativeRuntime)
            }
        };
        if !runtime_selectors.is_empty() {
            return Ok(runtime_selectors
                .into_iter()
                .map(|selector| ProcessEvidence {
                    selector,
                    source: runtime_source,
                })
                .collect());
        }

        Ok(self
            .resume_selectors(process)
            .into_iter()
            .map(|id| ProcessEvidence {
                selector: ProcessSelector::Id(id),
                source: AssociationEvidence::ResumeArgument,
            })
            .collect())
    }

    pub(super) fn discover(
        self,
        home: &Path,
        include_empty: bool,
        include_archived: bool,
        sessions: &mut Vec<AgentSession>,
        warnings: &mut Vec<String>,
    ) -> Result<()> {
        match self {
            Self::ClaudeCode => scan_claude(home, sessions, warnings),
            Self::Codex => scan_codex(home, sessions, warnings),
            Self::Goose => Ok(()),
            Self::GeminiCli => scan_gemini(home, sessions, warnings),
            Self::OpenCode => scan_opencode(home, sessions, warnings),
            Self::Pi => scan_pi(home, sessions, warnings),
            Self::OhMyPi => scan_oh_my_pi(home, sessions, warnings),
            Self::Cursor => scan_cursor(home, include_empty, include_archived, sessions, warnings),
            Self::Grok => scan_grok(home, include_empty, sessions, warnings),
        }
    }

    pub(super) fn load(self, home: &Path, selected: &AgentSession) -> Result<SessionDetail> {
        let loaded = match self {
            Self::Codex => detail::codex_detail(&selected.path)?,
            Self::ClaudeCode | Self::Pi | Self::OhMyPi => {
                detail::nested_jsonl_detail(&selected.path, &self.kind())?
            }
            Self::GeminiCli => detail::gemini_detail(&selected.path)?,
            Self::OpenCode => detail::opencode_detail(home, &selected.id)?,
            Self::Cursor => detail::cursor_detail(&selected.path, &selected.id)?,
            Self::Grok => detail::grok_detail(&selected.path)?,
            Self::Goose => bail!("Goose session detail loading is not implemented"),
        };
        let mut session = selected.clone();
        session.tokens = loaded.tokens;
        session.cost_usd = loaded.cost_usd;
        Ok(SessionDetail {
            session,
            messages: loaded.messages,
        })
    }

    pub(super) fn delete(
        self,
        home: &Path,
        selected: &AgentSession,
        mut files: BTreeSet<PathBuf>,
    ) -> Result<DeletionSummary> {
        validate_storage_identifier(&selected.id, "session ID")?;
        let mut directories = BTreeSet::new();
        match self {
            Self::Codex => collect_codex_artifacts(home, selected, &mut files)?,
            Self::ClaudeCode => {
                collect_claude_artifacts(home, selected, &mut files, &mut directories)?;
            }
            Self::OpenCode => {
                collect_opencode_artifacts(home, selected, &mut files, &mut directories)?;
            }
            Self::Cursor => {
                collect_cursor_artifacts(home, selected, &mut files)?;
                files.remove(&selected.path);
            }
            Self::Grok => collect_grok_artifacts(selected, &mut directories),
            Self::GeminiCli | Self::Pi | Self::OhMyPi | Self::Goose => {}
        }

        let roots = self.storage_roots(home);
        validate_deletion_targets(&roots, &files, &directories)?;
        let mut trash = super::trash::TrashBatch::create(home, selected.kind.slug(), &selected.id)?;
        let index_records = match self {
            Self::Codex => delete_codex_index_records(home, &selected.id)?,
            Self::ClaudeCode => delete_claude_index_records(home, &selected.id)?,
            Self::Cursor => delete_cursor_index_records(home, &selected.id, &mut trash)?,
            Self::GeminiCli
            | Self::OpenCode
            | Self::Pi
            | Self::OhMyPi
            | Self::Goose
            | Self::Grok => 0,
        };

        let mut summary = DeletionSummary {
            index_records,
            ..DeletionSummary::default()
        };
        for file in files {
            remove_file_if_present(&file, &mut summary, &mut trash)?;
        }
        let mut directories: Vec<_> = directories.into_iter().collect();
        directories.sort_by_key(|directory| std::cmp::Reverse(directory.components().count()));
        for directory in directories {
            remove_tree_if_present(&directory, &mut summary, &mut trash)?;
        }
        super::trash::finish(trash)?;
        Ok(summary)
    }

    fn storage_roots(self, home: &Path) -> Vec<PathBuf> {
        match self {
            Self::Codex => vec![
                home.join(".codex/sessions"),
                home.join(".codex/shell_snapshots"),
            ],
            Self::ClaudeCode => vec![home.join(".claude")],
            Self::Goose => vec![home.join(".goose")],
            Self::GeminiCli => vec![home.join(".gemini/tmp")],
            Self::OpenCode => vec![home.join(".local/share/opencode/storage")],
            Self::Pi => vec![home.join(".pi/agent/sessions")],
            Self::OhMyPi => vec![home.join(".omp/agent/sessions")],
            Self::Cursor => cursor_global_storage_dirs(home),
            Self::Grok => vec![grok_sessions_root(home)],
        }
    }

    pub(super) fn resume_command(self, id: &str) -> NativeResumeCommand {
        let (program, args): (&str, Vec<String>) = match self {
            Self::ClaudeCode => ("claude", vec!["--resume".to_owned(), id.to_owned()]),
            Self::Codex => ("codex", vec!["resume".to_owned(), id.to_owned()]),
            Self::Goose => (
                "goose",
                vec!["session".to_owned(), "resume".to_owned(), id.to_owned()],
            ),
            Self::GeminiCli => ("gemini", vec!["--resume".to_owned(), id.to_owned()]),
            Self::OpenCode => ("opencode", vec!["--session".to_owned(), id.to_owned()]),
            Self::Pi => ("pi", vec!["--session".to_owned(), id.to_owned()]),
            Self::OhMyPi => ("omp", vec!["--resume".to_owned(), id.to_owned()]),
            Self::Cursor => ("cursor-agent", vec!["--resume".to_owned(), id.to_owned()]),
            Self::Grok => ("grok", vec!["--resume".to_owned(), id.to_owned()]),
        };
        NativeResumeCommand {
            program: program.to_owned(),
            args,
        }
    }

    fn resume_selectors(self, process: &ProcessSnapshot) -> Vec<String> {
        let arguments = native_arguments(process);
        let mut selectors = match self {
            Self::Codex => arguments
                .strip_prefix(["resume".to_owned()].as_slice())
                .and_then(|arguments| arguments.first())
                .filter(|selector| !selector.starts_with('-'))
                .cloned()
                .into_iter()
                .collect(),
            Self::ClaudeCode | Self::GeminiCli | Self::OhMyPi | Self::Cursor | Self::Goose => {
                flag_values(arguments, "--resume")
            }
            Self::Grok => {
                let mut values = flag_values(arguments, "--resume");
                values.extend(flag_values(arguments, "-r"));
                values
            }
            Self::OpenCode | Self::Pi => flag_values(arguments, "--session"),
        };
        selectors.sort();
        selectors.dedup();
        selectors
    }
}

fn grok_process_evidence(
    home: &Path,
    process: &ProcessSnapshot,
    adapter: ProviderAdapter,
) -> Result<Vec<ProcessEvidence>> {
    let runtime_ids = runtime_grok_session_ids(home, process)?;
    if !runtime_ids.is_empty() {
        return Ok(runtime_ids
            .into_iter()
            .map(|id| ProcessEvidence {
                selector: ProcessSelector::Id(id),
                source: AssociationEvidence::NativeRuntime,
            })
            .collect());
    }

    let open_ids = grok_open_session_ids(home, process);
    if !open_ids.is_empty() {
        return Ok(open_ids
            .into_iter()
            .map(|id| ProcessEvidence {
                selector: ProcessSelector::Id(id),
                source: AssociationEvidence::OpenSessionFile,
            })
            .collect());
    }

    Ok(adapter
        .resume_selectors(process)
        .into_iter()
        .map(|id| ProcessEvidence {
            selector: ProcessSelector::Id(id),
            source: AssociationEvidence::ResumeArgument,
        })
        .collect())
}

fn grok_open_session_ids(home: &Path, process: &ProcessSnapshot) -> Vec<String> {
    let root = grok_sessions_root(home);
    let resolved_root = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
    let mut ids = Vec::new();
    for path in open_file_paths(process.pid) {
        if let Some(id) = grok_session_id_for_open_path(&root, &resolved_root, &path) {
            ids.push(id);
        }
    }
    ids.sort();
    ids.dedup();
    ids
}

fn grok_session_id_for_open_path(root: &Path, resolved_root: &Path, path: &Path) -> Option<String> {
    let relative = path
        .strip_prefix(root)
        .ok()
        .map(Path::to_path_buf)
        .or_else(|| {
            std::fs::canonicalize(path).ok().and_then(|canonical| {
                canonical
                    .strip_prefix(resolved_root)
                    .ok()
                    .map(Path::to_path_buf)
            })
        })?;
    let mut components = relative.components();
    let _group = components.next()?;
    let id = components.next()?;
    id.as_os_str().to_str().map(str::to_owned)
}

fn native_arguments(process: &ProcessSnapshot) -> &[String] {
    let executable = process
        .executable
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let offset = if matches!(executable, "node" | "node.exe" | "bun" | "bun.exe") {
        2
    } else {
        1
    };
    process.command.get(offset..).unwrap_or_default()
}

fn flag_values(arguments: &[String], flag: &str) -> Vec<String> {
    let assigned = format!("{flag}=");
    let mut values = Vec::new();
    for (index, argument) in arguments.iter().enumerate() {
        if argument == flag {
            if let Some(value) = arguments
                .get(index + 1)
                .filter(|value| !value.starts_with('-'))
            {
                values.push(value.clone());
            }
        } else if let Some(value) = argument.strip_prefix(&assigned)
            && !value.is_empty()
        {
            values.push(value.to_owned());
        }
    }
    values
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeResumeCommand {
    pub program: String,
    pub args: Vec<String>,
}

#[cfg(test)]
mod tests;
