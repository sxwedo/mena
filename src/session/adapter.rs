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
    collect_opencode_artifacts, cursor_global_storage_dirs, delete_claude_index_records,
    delete_codex_index_records, delete_cursor_index_records, runtime_claude_session_ids,
    scan_claude, scan_codex, scan_cursor, scan_gemini, scan_oh_my_pi, scan_opencode, scan_pi,
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
            ProcessSelector::Path(path) => paths_equivalent(&session.path, path),
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
}

impl ProviderAdapter {
    /// Providers with a native local session catalog, in stable display order.
    pub(super) const SESSION_CATALOG: [Self; 8] = [
        Self::Codex,
        Self::ClaudeCode,
        Self::Goose,
        Self::GeminiCli,
        Self::OpenCode,
        Self::Pi,
        Self::OhMyPi,
        Self::Cursor,
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
            AgentKind::Custom(_) => None,
        }
    }

    const fn kind(self) -> AgentKind {
        match self {
            Self::ClaudeCode => AgentKind::ClaudeCode,
            Self::Codex => AgentKind::Codex,
            Self::Goose => AgentKind::Goose,
            Self::GeminiCli => AgentKind::GeminiCli,
            Self::OpenCode => AgentKind::OpenCode,
            Self::Pi => AgentKind::Pi,
            Self::OhMyPi => AgentKind::OhMyPi,
            Self::Cursor => AgentKind::Cursor,
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
        let (runtime_selectors, runtime_source) = match self {
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

    pub(super) fn discover(self, home: &Path, sessions: &mut Vec<AgentSession>) -> Result<()> {
        match self {
            Self::ClaudeCode => scan_claude(home, sessions),
            Self::Codex => scan_codex(home, sessions),
            Self::Goose => Ok(()),
            Self::GeminiCli => scan_gemini(home, sessions),
            Self::OpenCode => scan_opencode(home, sessions),
            Self::Pi => scan_pi(home, sessions),
            Self::OhMyPi => scan_oh_my_pi(home, sessions),
            Self::Cursor => scan_cursor(home, sessions),
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
            Self::GeminiCli | Self::Pi | Self::OhMyPi | Self::Goose => {}
        }

        let roots = self.storage_roots(home);
        validate_deletion_targets(&roots, &files, &directories)?;
        let index_records = match self {
            Self::Codex => delete_codex_index_records(home, &selected.id)?,
            Self::ClaudeCode => delete_claude_index_records(home, &selected.id)?,
            Self::Cursor => delete_cursor_index_records(home, &selected.id)?,
            Self::GeminiCli | Self::OpenCode | Self::Pi | Self::OhMyPi | Self::Goose => 0,
        };

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
            Self::OpenCode | Self::Pi => flag_values(arguments, "--session"),
        };
        selectors.sort();
        selectors.dedup();
        selectors
    }
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
mod tests {
    use std::path::PathBuf;

    use super::{super::native_resume_command, ProviderAdapter};
    use crate::{AgentKind, ProcessSnapshot};

    fn process(executable: &str, command: &[&str]) -> ProcessSnapshot {
        ProcessSnapshot {
            pid: 42,
            parent_pid: Some(1),
            executable: PathBuf::from(executable),
            command: command.iter().map(ToString::to_string).collect(),
            cwd: Some(PathBuf::from("/work/project")),
            started_at: 100,
            run_time: 1,
            cpu_percent: 0.0,
            memory_bytes: 1,
            status: "running".to_owned(),
        }
    }

    #[test]
    fn every_session_provider_has_one_static_adapter() {
        let kinds = [
            AgentKind::Codex,
            AgentKind::ClaudeCode,
            AgentKind::Goose,
            AgentKind::GeminiCli,
            AgentKind::OpenCode,
            AgentKind::Pi,
            AgentKind::OhMyPi,
            AgentKind::Cursor,
        ];
        assert_eq!(ProviderAdapter::SESSION_CATALOG.len(), kinds.len());
        for (adapter, kind) in ProviderAdapter::SESSION_CATALOG.into_iter().zip(kinds) {
            assert!(adapter.matches(&kind));
        }
        assert!(ProviderAdapter::from_kind(&AgentKind::Cursor).is_some());
        assert!(ProviderAdapter::from_kind(&AgentKind::Custom("custom".to_owned())).is_none());
    }

    #[test]
    fn native_resume_commands_are_argv_not_shell_fragments() {
        let cases = [
            (
                AgentKind::ClaudeCode,
                "claude",
                vec!["--resume", "session-id"],
            ),
            (AgentKind::Codex, "codex", vec!["resume", "session-id"]),
            (
                AgentKind::Goose,
                "goose",
                vec!["session", "resume", "session-id"],
            ),
            (
                AgentKind::GeminiCli,
                "gemini",
                vec!["--resume", "session-id"],
            ),
            (
                AgentKind::OpenCode,
                "opencode",
                vec!["--session", "session-id"],
            ),
            (AgentKind::Pi, "pi", vec!["--session", "session-id"]),
            (AgentKind::OhMyPi, "omp", vec!["--resume", "session-id"]),
            (
                AgentKind::Cursor,
                "cursor-agent",
                vec!["--resume", "session-id"],
            ),
        ];

        for (kind, program, args) in cases {
            let command = native_resume_command(&kind, "session-id").expect("resume command");
            assert_eq!(command.program, program);
            assert_eq!(command.args, args);
            assert!(!command.program.contains(' '));
        }
    }

    #[test]
    fn resume_selectors_parse_only_provider_native_argv_positions() {
        let cases = [
            (
                ProviderAdapter::ClaudeCode,
                process("claude", &["claude", "--resume", "claude-id"]),
                vec!["claude-id"],
            ),
            (
                ProviderAdapter::Codex,
                process("codex", &["codex", "resume", "codex-id"]),
                vec!["codex-id"],
            ),
            (
                ProviderAdapter::GeminiCli,
                process("gemini", &["gemini", "--resume=gemini-id"]),
                vec!["gemini-id"],
            ),
            (
                ProviderAdapter::OpenCode,
                process("opencode", &["opencode", "--session", "opencode-id"]),
                vec!["opencode-id"],
            ),
            (
                ProviderAdapter::Pi,
                process("pi", &["pi", "--session=pi-id"]),
                vec!["pi-id"],
            ),
            (
                ProviderAdapter::OhMyPi,
                process("omp", &["omp", "--resume", "omp-id"]),
                vec!["omp-id"],
            ),
        ];

        for (adapter, process, expected) in cases {
            assert_eq!(adapter.resume_selectors(&process), expected);
        }

        assert!(
            ProviderAdapter::Codex
                .resume_selectors(&process(
                    "codex",
                    &["codex", "exec", "resume", "not-a-selector"]
                ))
                .is_empty()
        );
        assert_eq!(
            ProviderAdapter::ClaudeCode.resume_selectors(&process(
                "node",
                &[
                    "node",
                    "/lib/node_modules/@anthropic-ai/claude-code/cli.js",
                    "--resume",
                    "wrapped-id",
                ],
            )),
            vec!["wrapped-id"]
        );
    }
}
