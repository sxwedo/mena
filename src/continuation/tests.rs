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
    assert_eq!(
        continuation_targets(&AgentKind::Grok),
        vec![
            ContinuationTarget {
                kind: AgentKind::OhMyPi,
                method: ContinuationMethod::Handoff,
            },
            ContinuationTarget {
                kind: AgentKind::ClaudeCode,
                method: ContinuationMethod::Handoff,
            },
            ContinuationTarget {
                kind: AgentKind::Codex,
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
        (AgentKind::OhMyPi, "omp"),
    ] {
        let spec = continuation_launch_spec(&AgentKind::Grok, &target, Some(handoff))
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
        is_subagent: false,
        parent_session_id: None,
    };
    let target = ContinuationTarget {
        kind: AgentKind::Codex,
        method: ContinuationMethod::Handoff,
    };
    let prepared = prepare_continuation(&session, &target, || {
        Ok(SessionDetail {
            session: session.clone(),
            messages: vec![SessionMessage {
                fork_point: None,
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
        is_subagent: false,
        parent_session_id: None,
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

#[test]
fn message_fork_copies_only_the_selected_prefix_into_a_fresh_chat() {
    let project = tempdir().expect("project");
    let path = project.path().join("source.jsonl");
    std::fs::write(&path, "original transcript").expect("source");
    let session = AgentSession {
        kind: AgentKind::Codex,
        id: "source".to_owned(),
        title: None,
        project: Some(project.path().to_path_buf()),
        path: path.clone(),
        started_at: None,
        updated_at: 0,
        tokens: Some(900),
        cost_usd: Some(1.0),
        related_paths: BTreeSet::new(),
        is_subagent: false,
        parent_session_id: None,
    };
    let messages = ["FIRST-MARKER", "SELECTED-MARKER", "EXCLUDED-LATER-MARKER"]
        .into_iter()
        .map(|content| SessionMessage {
            fork_point: None,
            kind: SessionMessageKind::Assistant,
            timestamp: None,
            model: None,
            metrics: SessionMessageMetrics::default(),
            content: content.to_owned(),
        })
        .collect();
    let detail = SessionDetail { session, messages };
    let fork = super::prepare_message_fork(&detail, 1, &AgentKind::Codex)
        .expect("same-agent context fork");
    let text = std::fs::read_to_string(fork.handoff_path().expect("handoff")).expect("context");
    assert!(text.contains("FIRST-MARKER") && text.contains("SELECTED-MARKER"));
    assert!(!text.contains("EXCLUDED-LATER-MARKER"));
    assert_eq!(fork.command().program, "codex");
    assert!(
        !fork
            .command()
            .args
            .iter()
            .any(|argument| argument == "resume")
    );
    assert_eq!(
        std::fs::read_to_string(path).expect("source"),
        "original transcript"
    );
    assert!(super::prepare_message_fork(&detail, 3, &AgentKind::Codex).is_err());
}
