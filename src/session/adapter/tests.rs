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
        AgentKind::Grok,
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
        (AgentKind::Grok, "grok", vec!["--resume", "session-id"]),
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
        (
            ProviderAdapter::Grok,
            process("grok", &["grok", "--resume", "grok-id"]),
            vec!["grok-id"],
        ),
        (
            ProviderAdapter::Grok,
            process("grok", &["grok", "-r", "grok-short"]),
            vec!["grok-short"],
        ),
        (
            ProviderAdapter::Grok,
            process("grok", &["grok", "--resume=grok-eq"]),
            vec!["grok-eq"],
        ),
        (
            ProviderAdapter::Grok,
            process("grok", &["grok", "--resume"]),
            Vec::new(),
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
