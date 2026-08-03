use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use super::{
    AgentSession, DeletionSummary, SessionDetail, read_json_file, remove_file_if_present,
    remove_tree_if_present, validate_deletion_targets, validate_storage_identifier,
};
use crate::AgentKind;

mod detail;
mod storage;

use storage::{
    collect_claude_artifacts, collect_codex_artifacts, collect_opencode_artifacts,
    delete_claude_index_records, delete_codex_index_records, scan_claude, scan_codex, scan_gemini,
    scan_oh_my_pi, scan_opencode, scan_pi,
};

/// A built-in provider adapter selected without allocation or dynamic dispatch.
///
/// The enum is deliberately closed: adding a provider makes every capability
/// match non-exhaustive until discovery, detail, resume, and deletion semantics
/// have been considered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProviderAdapter {
    ClaudeCode,
    Codex,
    GeminiCli,
    OpenCode,
    Pi,
    OhMyPi,
    Cursor,
}

impl ProviderAdapter {
    /// Providers with a native local session catalog, in stable display order.
    pub(super) const SESSION_CATALOG: [Self; 6] = [
        Self::Codex,
        Self::ClaudeCode,
        Self::GeminiCli,
        Self::OpenCode,
        Self::Pi,
        Self::OhMyPi,
    ];

    pub(super) const fn from_kind(kind: &AgentKind) -> Option<Self> {
        match kind {
            AgentKind::ClaudeCode => Some(Self::ClaudeCode),
            AgentKind::Codex => Some(Self::Codex),
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

    pub(super) fn discover(self, home: &Path, sessions: &mut Vec<AgentSession>) -> Result<()> {
        match self {
            Self::ClaudeCode => scan_claude(home, sessions),
            Self::Codex => scan_codex(home, sessions),
            Self::GeminiCli => scan_gemini(home, sessions),
            Self::OpenCode => scan_opencode(home, sessions),
            Self::Pi => scan_pi(home, sessions),
            Self::OhMyPi => scan_oh_my_pi(home, sessions),
            Self::Cursor => bail!("Cursor does not expose a supported local session catalog"),
        }
    }

    pub(super) fn usage(self, home: &Path, session: &AgentSession) -> Result<detail::Usage> {
        match self {
            Self::Codex | Self::ClaudeCode | Self::Pi | Self::OhMyPi => {
                detail::jsonl_usage(&session.path, &self.kind())
            }
            Self::GeminiCli => {
                let usage = read_json_file(&session.path)?
                    .as_ref()
                    .map_or((None, None), detail::gemini_usage);
                Ok(usage)
            }
            Self::OpenCode => detail::opencode_usage(home, &session.id),
            Self::Cursor => Ok((None, None)),
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
            Self::Cursor => detail::LoadedSession::default(),
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
            Self::GeminiCli | Self::Pi | Self::OhMyPi => {}
            Self::Cursor => bail!("Cursor sessions do not support local deletion"),
        }

        let roots = self.storage_roots(home);
        validate_deletion_targets(&roots, &files, &directories)?;
        let index_records = match self {
            Self::Codex => delete_codex_index_records(home, &selected.id)?,
            Self::ClaudeCode => delete_claude_index_records(home, &selected.id)?,
            Self::GeminiCli | Self::OpenCode | Self::Pi | Self::OhMyPi | Self::Cursor => 0,
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
            Self::GeminiCli => vec![home.join(".gemini/tmp")],
            Self::OpenCode => vec![home.join(".local/share/opencode/storage")],
            Self::Pi => vec![home.join(".pi/agent/sessions")],
            Self::OhMyPi => vec![home.join(".omp/agent/sessions")],
            Self::Cursor => Vec::new(),
        }
    }

    pub(super) fn resume_command(self, id: &str) -> NativeResumeCommand {
        let (program, args): (&str, Vec<String>) = match self {
            Self::ClaudeCode => ("claude", vec!["--resume".to_owned(), id.to_owned()]),
            Self::Codex => ("codex", vec!["resume".to_owned(), id.to_owned()]),
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeResumeCommand {
    pub program: String,
    pub args: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::{super::native_resume_command, ProviderAdapter};
    use crate::AgentKind;

    #[test]
    fn every_session_provider_has_one_static_adapter() {
        let kinds = [
            AgentKind::Codex,
            AgentKind::ClaudeCode,
            AgentKind::GeminiCli,
            AgentKind::OpenCode,
            AgentKind::Pi,
            AgentKind::OhMyPi,
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
}
