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
    /// List saved sessions, including sessions without a running process
    #[command(visible_alias = "ss")]
    Sessions(SessionsArgs),
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
pub struct SessionsArgs {
    /// Filter by provider: claude, codex, cursor, gemini, opencode, pi, or omp
    #[arg(long)]
    pub provider: Option<String>,
    /// Show only the most recently updated N sessions
    #[arg(long)]
    pub limit: Option<usize>,
    /// Emit stable machine-readable JSON
    #[arg(long)]
    pub json: bool,
    /// Include messageless empty draft sessions
    #[arg(long)]
    pub include_empty: bool,
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
        AgentCommand::Sessions(args) => controller::run_sessions(&args, settings),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{AgentKind, ProcessSnapshot};
    use crate::process::recognize_agent;

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
}
