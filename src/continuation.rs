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
mod tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::Path;

    use tempfile::tempdir;

    use super::{
        ContinuationMethod, ContinuationTarget, continuation_launch_spec, continuation_targets,
        prepare_continuation,
    };
    use crate::AgentKind;
    use crate::session::{
        AgentSession, SessionDetail, SessionMessage, SessionMessageKind, SessionMessageMetrics,
    };

    #[test]
    fn omp_uses_native_foreign_import_for_claude_and_codex_sessions() {
        for (source, flag) in [
            (AgentKind::ClaudeCode, "--from-claude"),
            (AgentKind::Codex, "--from-codex"),
        ] {
            let spec = continuation_launch_spec(&source, &AgentKind::OhMyPi, None)
                .expect("OMP import launch spec");

            assert_eq!(spec.program, "omp");
            assert_eq!(spec.args, [flag]);
            assert!(!spec.args.iter().any(|arg| arg == "--resume"));
        }
    }

    #[test]
    fn continuation_targets_exclude_the_source_and_describe_the_transfer_method() {
        assert_eq!(
            continuation_targets(&AgentKind::ClaudeCode),
            vec![
                ContinuationTarget {
                    kind: AgentKind::OhMyPi,
                    method: ContinuationMethod::NativeImport,
                },
                ContinuationTarget {
                    kind: AgentKind::Codex,
                    method: ContinuationMethod::Handoff,
                },
            ]
        );
        assert_eq!(
            continuation_targets(&AgentKind::Codex),
            vec![
                ContinuationTarget {
                    kind: AgentKind::OhMyPi,
                    method: ContinuationMethod::NativeImport,
                },
                ContinuationTarget {
                    kind: AgentKind::ClaudeCode,
                    method: ContinuationMethod::Handoff,
                },
            ]
        );
    }

    #[test]
    fn handoff_launches_a_fresh_target_with_the_private_markdown_path() {
        let handoff = Path::new("/work/project/mena-handoff.md");
        for (target, program) in [
            (AgentKind::ClaudeCode, "claude"),
            (AgentKind::Codex, "codex"),
        ] {
            let spec = continuation_launch_spec(&AgentKind::OpenCode, &target, Some(handoff))
                .expect("handoff launch spec");

            assert_eq!(spec.program, program);
            assert_eq!(spec.args.len(), 1);
            assert!(spec.args[0].contains("/work/project/mena-handoff.md"));
            assert!(spec.args[0].contains("Continue the work"));
            assert!(!spec.args.iter().any(|arg| arg.contains("--resume")));
        }
    }

    #[test]
    fn prepared_handoff_is_private_contains_context_and_is_removed_after_use() {
        let project = tempdir().expect("project directory");
        let session = AgentSession {
            kind: AgentKind::ClaudeCode,
            id: "claude-session".to_owned(),
            title: Some("Continue the parser work".to_owned()),
            project: Some(project.path().to_path_buf()),
            path: project.path().join("source.jsonl"),
            started_at: None,
            updated_at: 1,
            tokens: None,
            cost_usd: None,
            related_paths: BTreeSet::new(),
        };
        let target = ContinuationTarget {
            kind: AgentKind::Codex,
            method: ContinuationMethod::Handoff,
        };
        let prepared = prepare_continuation(&session, &target, || {
            Ok(SessionDetail {
                session: session.clone(),
                messages: vec![SessionMessage {
                    kind: SessionMessageKind::User,
                    timestamp: None,
                    model: None,
                    metrics: SessionMessageMetrics::default(),
                    content: "the remaining task is parser recovery".to_owned(),
                }],
            })
        })
        .expect("prepare handoff");
        let handoff_path = prepared.handoff_path().expect("handoff path").to_path_buf();

        assert!(handoff_path.starts_with(project.path()));
        let markdown = fs::read_to_string(&handoff_path).expect("read handoff");
        assert!(markdown.contains("# Mena Cross-Agent Handoff"));
        assert!(markdown.contains("Source: `claude:claude-session`"));
        assert!(markdown.contains("Target: `codex`"));
        assert!(markdown.contains("the remaining task is parser recovery"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            assert_eq!(
                fs::metadata(&handoff_path)
                    .expect("handoff metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }

        drop(prepared);
        assert!(!handoff_path.exists());
    }

    #[test]
    fn native_import_does_not_load_or_copy_the_transcript() {
        let session = AgentSession {
            kind: AgentKind::Codex,
            id: "codex-session".to_owned(),
            title: None,
            project: Some(Path::new("/work/project").to_path_buf()),
            path: Path::new("/logs/codex.jsonl").to_path_buf(),
            started_at: None,
            updated_at: 1,
            tokens: None,
            cost_usd: None,
            related_paths: BTreeSet::new(),
        };
        let target = ContinuationTarget {
            kind: AgentKind::OhMyPi,
            method: ContinuationMethod::NativeImport,
        };

        let prepared =
            prepare_continuation(&session, &target, || panic!("native import must stay lazy"))
                .expect("prepare native import");

        assert_eq!(prepared.command().program, "omp");
        assert_eq!(prepared.command().args, ["--from-codex"]);
        assert!(prepared.handoff_path().is_none());
    }
}
