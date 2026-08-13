//! Local-first developer-agent launcher and catalog for sessions, Skills, and MCP servers.

mod clipboard;
mod controller;
mod export;
mod fs;
mod mcp;
mod process;
mod session;
pub mod settings;
mod skill;
mod tui;
pub mod ui;
mod view;

use anyhow::Result;
use clap::{Args, Subcommand};
pub use mcp::{
    McpAuthentication, McpCatalog, McpDetail, McpListCapability, McpProbe, McpProbeStatus,
    McpPromptArgument, McpRegistration, McpResourceCapability, McpRuntimePrompt,
    McpRuntimeResource, McpRuntimeResourceTemplate, McpRuntimeTool, McpServerCapabilities,
    McpServerIdentity, McpSourceFormat, McpTimeouts, McpToolAnnotations, McpToolPolicy,
    McpTransport, McpValueBinding, McpValueSource,
};
pub use process::{AgentKind, ProcessSnapshot};
pub use settings::Settings;
pub use skill::{AgentSkill, SkillCatalog, SkillDetail};

/// Mena command-line arguments.
#[derive(Debug, Args)]
pub struct AgentArgs {
    #[command(subcommand)]
    pub command: AgentCommand,
}

/// Local developer-agent process and session operations.
#[derive(Debug, Subcommand)]
pub enum AgentCommand {
    /// Select and launch a developer agent process in the current directory
    #[command(visible_alias = "ag")]
    Agent(AgentLaunchArgs),
    /// Manage mena configuration
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// List saved sessions, including sessions without a running process
    #[command(visible_alias = "ss")]
    Sessions(SessionsArgs),
    /// Inspect or list available developer agent skills
    #[command(visible_alias = "sk")]
    Skills(SkillsArgs),
    /// Inspect MCP server registrations and their metadata
    Mcp(McpArgs),
}

/// Mena configuration operations.
#[derive(Debug, Clone, Subcommand)]
pub enum ConfigCommand {
    /// Create ~/.config/mena/config.toml with restrictive permissions
    Init,
}
#[derive(Debug, Clone, Args)]
pub struct AgentLaunchArgs {
    /// Provider to launch (claude, codex, goose, gemini, opencode, pi, omp, cursor, or custom)
    pub provider: Option<String>,
    /// Force launching a fresh new session
    #[arg(long, short = 'n')]
    pub fresh: bool,
    /// Resume the latest saved session in the current directory
    #[arg(long, short = 'r')]
    pub resume: bool,
    /// Resume a specific session by ID or prefix in the current directory
    #[arg(long)]
    pub session: Option<String>,
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

#[derive(Debug, Clone, Args)]
pub struct SkillsArgs {
    #[command(subcommand)]
    pub command: Option<SkillSubcommand>,
    /// Filter by provider: claude, codex, cursor, opencode, or omp
    #[arg(long)]
    pub provider: Option<String>,
    /// Filter by scope: global or workspace
    #[arg(long)]
    pub scope: Option<String>,
    /// Emit stable machine-readable JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Subcommand)]
pub enum SkillSubcommand {
    /// Inspect details of a specific skill by name
    Inspect {
        /// Skill name to inspect
        name: String,
        /// Emit stable machine-readable JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, Args)]
pub struct McpArgs {
    #[command(subcommand)]
    pub command: Option<McpSubcommand>,
    /// Filter by client provider: claude, codex, cursor, gemini, goose, omp, opencode, or pi
    #[arg(long)]
    pub provider: Option<String>,
    /// Filter by configuration scope: user, local, project, plugin, profile, managed, or shared
    #[arg(long)]
    pub scope: Option<String>,
    /// Filter by configuration source path or suffix
    #[arg(long)]
    pub source: Option<String>,
    /// Emit stable machine-readable JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Subcommand)]
pub enum McpSubcommand {
    /// Inspect one uniquely identified MCP registration
    Inspect {
        /// MCP server name or provider:scope:name selector
        name: String,
        /// Connect to the server and discover protocol metadata; never calls tools
        #[arg(long)]
        probe: bool,
        /// Maximum live-probe duration in seconds
        #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(u64).range(1..=300))]
        timeout: u64,
        /// Emit stable machine-readable JSON
        #[arg(long)]
        json: bool,
    },
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
            ConfigCommand::Init => {
                let path = settings::ensure_default_config()?;
                ui::success(format!("created {} (mode 0600)", path.display()));
                Ok(())
            }
        },
        AgentCommand::Agent(args) => controller::run_agent(&args, settings),
        AgentCommand::Sessions(args) => controller::run_sessions(&args, settings),
        AgentCommand::Skills(args) => controller::run_skills(&args, settings),
        AgentCommand::Mcp(args) => controller::run_mcp(&args),
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
