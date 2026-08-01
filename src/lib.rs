//! Local-first process and session control for developer agents.

mod clipboard;
mod controller;
mod export;
mod fs;
mod process;
mod session;
pub mod settings;
mod tui;
pub mod ui;
mod view;

use anyhow::Result;
use clap::{Args, Subcommand};
pub use process::{AgentKind, ProcessSnapshot};
pub use settings::Settings;

/// Mena command-line arguments.
#[derive(Debug, Args)]
pub struct AgentArgs {
    #[command(subcommand)]
    pub command: AgentCommand,
}

/// Local developer-agent process and session operations.
#[derive(Debug, Subcommand)]
pub enum AgentCommand {
    /// Manage mena configuration
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// List running developer-agent processes
    Ps(PsArgs),
    /// Refresh a live resource view of developer-agent processes
    Top(TopArgs),
    /// Show process and session details
    Inspect(TargetArgs),
    /// Show the tail of a local agent session log
    Logs(LogsArgs),
    /// List saved sessions, including sessions without a running process
    Sessions(SessionsArgs),
    /// Gracefully terminate a running agent process
    Stop(StopArgs),
    /// Resume a saved agent session with its native CLI
    Resume(ResumeArgs),
}

/// Mena configuration operations.
#[derive(Debug, Clone, Subcommand)]
pub enum ConfigCommand {
    /// Create ~/.config/mena/config.toml with restrictive permissions
    Init {
        /// Import [agent.custom] entries from ~/.config/clix/config.toml
        #[arg(long)]
        import_clix: bool,
    },
}

#[derive(Debug, Clone, Args)]
pub struct PsArgs {
    /// Emit stable machine-readable JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Args)]
pub struct TopArgs {
    /// Refresh interval in seconds
    #[arg(short, long, default_value_t = 2)]
    pub interval: u64,
    /// Stop after this many refreshes (defaults to unlimited on a terminal)
    #[arg(long)]
    pub iterations: Option<u64>,
}

#[derive(Debug, Clone, Args)]
pub struct TargetArgs {
    /// PID, provider:PID, session ID, or provider:session ID
    pub target: String,
    /// Emit stable machine-readable JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Args)]
pub struct LogsArgs {
    /// PID, provider:PID, session ID, or provider:session ID
    pub target: String,
    /// Number of records to show
    #[arg(short = 'n', long, default_value_t = 50)]
    pub lines: usize,
    /// Print original JSONL records instead of a redacted event summary
    #[arg(long)]
    pub raw: bool,
}

#[derive(Debug, Clone, Args)]
pub struct SessionsArgs {
    /// Filter by provider: claude, codex, gemini, opencode, pi, or omp
    #[arg(long)]
    pub provider: Option<String>,
    /// Show only the most recently updated N sessions
    #[arg(long)]
    pub limit: Option<usize>,
    /// Emit stable machine-readable JSON
    #[arg(long)]
    pub json: bool,
    /// Print a non-interactive table even when attached to a terminal
    #[arg(long, conflicts_with = "json")]
    pub plain: bool,
}

#[derive(Debug, Clone, Args)]
pub struct StopArgs {
    /// PID or provider:PID of the live agent to stop
    pub target: String,
    /// Send a forceful kill signal instead of a graceful termination signal
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, Clone, Args)]
pub struct ResumeArgs {
    /// Session ID or provider:session ID
    pub target: Option<String>,
    /// List recent resumable sessions without starting one
    #[arg(long, conflicts_with_all = ["target", "last"])]
    pub list: bool,
    /// Resume the most recently updated session
    #[arg(long, conflicts_with_all = ["target", "list"])]
    pub last: bool,
    /// Maximum number of sessions shown by the list or picker
    #[arg(long, default_value_t = 30)]
    pub limit: usize,
}

/// Execute one `mena` command.
///
/// # Errors
///
/// Returns an actionable error when discovery fails, a target is ambiguous, or
/// an operation is unsupported by the selected agent.
pub fn run(args: AgentArgs, settings: &Settings) -> Result<()> {
    if !matches!(&args.command, AgentCommand::Config { .. }) {
        process::validate_custom_agents(&settings.agent.custom)?;
    }
    match args.command {
        AgentCommand::Config { command } => match command {
            ConfigCommand::Init { import_clix } => {
                let path = settings::ensure_default_config(import_clix)?;
                ui::success(format!("created {} (mode 0600)", path.display()));
                Ok(())
            }
        },
        AgentCommand::Ps(args) => controller::run_ps(&args, settings),
        AgentCommand::Top(args) => controller::run_top(&args, settings),
        AgentCommand::Inspect(args) => controller::run_inspect(&args, settings),
        AgentCommand::Logs(args) => controller::run_logs(&args, settings),
        AgentCommand::Sessions(args) => controller::run_sessions(&args, settings),
        AgentCommand::Stop(args) => controller::run_stop(&args, settings),
        AgentCommand::Resume(args) => controller::run_resume(&args, settings),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{AgentKind, ProcessSnapshot};
    use crate::process::{LiveAgent, recognize_agent};
    use crate::session::AgentSession;
    use crate::view::{AgentReport, render_process_table};

    fn process(executable: &str, command: &[&str]) -> ProcessSnapshot {
        ProcessSnapshot {
            pid: 42,
            parent_pid: Some(1),
            executable: PathBuf::from(executable),
            command: command.iter().map(ToString::to_string).collect(),
            cwd: Some(PathBuf::from("/work/project")),
            started_at: 1,
            run_time: 2,
            cpu_percent: 3.0,
            memory_bytes: 4,
            status: "sleeping".to_owned(),
        }
    }

    #[test]
    fn recognizes_supported_agent_executables_without_matching_desktop_helpers() {
        let cases = [
            ("/opt/bin/claude", AgentKind::ClaudeCode),
            ("/opt/bin/codex", AgentKind::Codex),
            ("/opt/bin/gemini", AgentKind::GeminiCli),
            ("/opt/bin/opencode", AgentKind::OpenCode),
            ("/opt/bin/pi", AgentKind::Pi),
            ("/opt/bin/omp", AgentKind::OhMyPi),
            ("/opt/bin/cursor-agent", AgentKind::Cursor),
        ];

        for (executable, expected) in cases {
            assert_eq!(
                recognize_agent(&process(executable, &[executable])),
                Some(expected),
                "failed to recognize {executable}"
            );
        }

        assert_eq!(
            recognize_agent(&process(
                "/Applications/Cursor.app/Contents/MacOS/Cursor",
                &["Cursor"]
            )),
            None
        );
        assert_eq!(
            recognize_agent(&process(
                "/Applications/ChatGPT.app/Helpers/Codex (Renderer)",
                &["Codex (Renderer)"]
            )),
            None
        );
    }

    #[test]
    fn recognizes_node_wrappers_by_package_marker() {
        assert_eq!(
            recognize_agent(&process(
                "/usr/bin/node",
                &["node", "/lib/node_modules/@anthropic-ai/claude-code/cli.js"]
            )),
            Some(AgentKind::ClaudeCode)
        );
        assert_eq!(
            recognize_agent(&process(
                "/usr/bin/node",
                &["node", "/lib/node_modules/@google/gemini-cli/dist/index.js"]
            )),
            Some(AgentKind::GeminiCli)
        );
        assert_eq!(
            recognize_agent(&process(
                "/usr/bin/node",
                &[
                    "node",
                    "/lib/node_modules/@oh-my-pi/pi-coding-agent/dist/cli.js"
                ]
            )),
            Some(AgentKind::OhMyPi)
        );
    }

    #[test]
    fn process_table_exposes_stable_selectors_and_unknown_usage_honestly() {
        let agent = LiveAgent {
            kind: AgentKind::Codex,
            process: process("/opt/bin/codex", &["codex"]),
        };
        let report = AgentReport {
            agent,
            session: Some(AgentSession {
                kind: AgentKind::Codex,
                id: "session-id".to_owned(),
                title: Some("Fix rendering".to_owned()),
                project: Some(PathBuf::from("/work/actual-project")),
                path: PathBuf::from("/tmp/session.jsonl"),
                started_at: None,
                updated_at: 1,
                tokens: Some(12_345),
                cost_usd: None,
            }),
        };

        let rendered = render_process_table(&[report], false, None);

        assert!(rendered.contains("ID"));
        assert!(rendered.contains("AGENT"));
        assert!(rendered.contains("PROJECT"));
        assert!(rendered.contains("STATUS"));
        assert!(rendered.contains("DURATION"));
        assert!(rendered.contains("TOKENS"));
        assert!(rendered.contains("COST"));
        assert!(rendered.contains("codex:42"));
        assert!(rendered.contains("actual-project"));
        assert!(rendered.contains("12.3K"));
        assert!(rendered.contains("n/a"));
    }
}
