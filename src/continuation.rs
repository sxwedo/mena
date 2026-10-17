use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, bail};
use tempfile::NamedTempFile;

use crate::AgentKind;
use crate::session::{AgentSession, DetailScope, NativeResumeCommand, SessionDetail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContinuationMethod {
    NativeImport,
    Handoff,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContinuationTarget {
    pub kind: AgentKind,
    pub method: ContinuationMethod,
}

pub struct PreparedContinuation {
    command: NativeResumeCommand,
    handoff: Option<NamedTempFile>,
}

impl PreparedContinuation {
    pub const fn command(&self) -> &NativeResumeCommand {
        &self.command
    }

    pub fn handoff_path(&self) -> Option<&Path> {
        self.handoff.as_ref().map(NamedTempFile::path)
    }
}

pub fn continuation_targets(source: &AgentKind) -> Vec<ContinuationTarget> {
    [AgentKind::OhMyPi, AgentKind::ClaudeCode, AgentKind::Codex]
        .into_iter()
        .filter(|target| target != source)
        .map(|kind| {
            let method = if kind == AgentKind::OhMyPi
                && matches!(source, AgentKind::ClaudeCode | AgentKind::Codex)
            {
                ContinuationMethod::NativeImport
            } else {
                ContinuationMethod::Handoff
            };
            ContinuationTarget { kind, method }
        })
        .collect()
}

fn continuation_launch_spec(
    source: &AgentKind,
    target: &AgentKind,
    handoff_path: Option<&Path>,
) -> Result<NativeResumeCommand> {
    if let Some(handoff_path) = handoff_path {
        if !handoff_path.is_absolute() {
            bail!("handoff path must be absolute: {}", handoff_path.display());
        }
        let program = match target {
            AgentKind::ClaudeCode => "claude",
            AgentKind::Codex => "codex",
            AgentKind::OhMyPi => "omp",
            _ => bail!("{} does not support mena handoff launch", target.slug()),
        };
        let prompt = format!(
            "Continue the work from the {} session using the handoff at {}. Read it first, verify the current repository state, and continue from its pending work without assuming persisted tool state is still valid.",
            source.slug(),
            handoff_path.display()
        );
        return Ok(NativeResumeCommand {
            program: program.to_owned(),
            args: vec![prompt],
        });
    }
    if *target == AgentKind::OhMyPi {
        let flag = match source {
            AgentKind::ClaudeCode => "--from-claude",
            AgentKind::Codex => "--from-codex",
            _ => bail!("OMP cannot natively import {} sessions", source.slug()),
        };
        return Ok(NativeResumeCommand {
            program: "omp".to_owned(),
            args: vec![flag.to_owned()],
        });
    }
    bail!(
        "cross-agent continuation from {} to {} is not implemented",
        source.slug(),
        target.slug()
    )
}

pub fn prepare_continuation(
    session: &AgentSession,
    target: &ContinuationTarget,
    load_detail: impl FnOnce() -> Result<SessionDetail>,
) -> Result<PreparedContinuation> {
    if target.kind == session.kind {
        bail!("use native resume to continue with the original agent");
    }
    match target.method {
        ContinuationMethod::NativeImport => Ok(PreparedContinuation {
            command: continuation_launch_spec(&session.kind, &target.kind, None)?,
            handoff: None,
        }),
        ContinuationMethod::Handoff => {
            let project = session.project.as_deref().with_context(|| {
                format!(
                    "cannot create a handoff for {} without a project",
                    session.target()
                )
            })?;
            if !project.is_dir() {
                bail!(
                    "cannot create a handoff for {} because its project directory no longer exists: {}",
                    session.target(),
                    project.display()
                );
            }
            let detail =
                load_detail().context("failed to load the selected session for handoff")?;
            let transcript =
                crate::export::render_session_detail_markdown(&detail, DetailScope::All);
            let markdown = format!(
                "# Mena Cross-Agent Handoff\n\n- Source: `{}`\n- Target: `{}`\n- Transfer: handoff to a fresh session\n\n## Continuation Contract\n\nRead the persisted context below, then verify the current repository state before acting. Treat tool results, process state, permissions, hooks, and uncommitted files as historical evidence rather than guaranteed current state.\n\n## Persisted Session Context\n\n{transcript}",
                session.target(),
                target.kind.slug()
            );
            let mut builder = tempfile::Builder::new();
            builder.prefix(".mena-handoff-").suffix(".md");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;

                builder.permissions(std::fs::Permissions::from_mode(0o600));
            }
            let mut handoff = builder.tempfile_in(project).with_context(|| {
                format!(
                    "failed to create a temporary handoff in {}",
                    project.display()
                )
            })?;
            handoff
                .write_all(markdown.as_bytes())
                .context("failed to write the temporary handoff")?;
            handoff
                .as_file_mut()
                .sync_all()
                .context("failed to sync the temporary handoff")?;
            let command =
                continuation_launch_spec(&session.kind, &target.kind, Some(handoff.path()))?;
            Ok(PreparedContinuation {
                command,
                handoff: Some(handoff),
            })
        }
    }
}

#[cfg(test)]
mod tests;
