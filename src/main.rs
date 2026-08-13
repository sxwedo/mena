use anyhow::Result;
use clap::Parser;
use mena::AgentArgs;

#[derive(Debug, Parser)]
#[command(
    name = "mena",
    author,
    version,
    about = "Launch developer agents and inspect running processes, sessions, skills, and MCP servers"
)]
struct Cli {
    #[command(flatten)]
    args: AgentArgs,
}

fn main() -> std::process::ExitCode {
    mena::ui::exit_code(execute())
}

fn execute() -> Result<()> {
    let cli = Cli::parse();
    let settings = mena::Settings::load()?;
    mena::run(cli.args, &settings)
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};
    use mena::AgentCommand;

    use super::Cli;

    #[test]
    fn command_tree_is_internally_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn config_init_has_no_user_options() {
        let command = Cli::command();
        let config = command
            .get_subcommands()
            .find(|subcommand| subcommand.get_name() == "config")
            .expect("config command");
        let init = config
            .get_subcommands()
            .find(|subcommand| subcommand.get_name() == "init")
            .expect("config init command");

        assert_eq!(init.get_arguments().count(), 0);
    }

    #[test]
    fn documented_commands_parse() {
        for invocation in [
            vec!["mena", "config", "init"],
            vec!["mena", "agent"],
            vec!["mena", "ag"],
            vec!["mena", "ag", "claude", "-n"],
            vec!["mena", "ag", "goose"],
            vec!["mena", "ag", "omp", "--resume"],
            vec!["mena", "ag", "codex", "--session", "session-123"],
            vec!["mena", "ps"],
            vec!["mena", "ps", "--json"],
            vec!["mena", "sessions"],
            vec!["mena", "ss"],
            vec![
                "mena",
                "sessions",
                "--provider",
                "claude",
                "--limit",
                "10",
                "--json",
            ],
            vec!["mena", "skills"],
            vec!["mena", "sk"],
            vec![
                "mena",
                "sk",
                "--provider",
                "claude",
                "--scope",
                "workspace",
                "--json",
            ],
            vec!["mena", "sk", "inspect", "ponytail", "--json"],
            vec!["mena", "mcp"],
            vec!["mena", "mcp", "open", "context7"],
            vec!["mena", "mcp", "--provider", "codex", "open", "context7"],
            vec![
                "mena",
                "mcp",
                "--provider",
                "codex",
                "--scope",
                "user",
                "--json",
            ],
            vec![
                "mena",
                "mcp",
                "--provider",
                "claude",
                "inspect",
                "context7",
                "--probe",
                "--timeout",
                "15",
                "--json",
            ],
            vec![
                "mena",
                "sk",
                "--provider",
                "codex",
                "--scope",
                "global",
                "inspect",
                "ponytail",
                "--json",
            ],
        ] {
            Cli::try_parse_from(&invocation)
                .unwrap_or_else(|error| panic!("failed to parse {invocation:?}: {error}"));
        }
    }

    #[test]
    fn ss_is_equivalent_to_sessions() {
        let cli = Cli::try_parse_from([
            "mena",
            "ss",
            "--provider",
            "codex",
            "--limit",
            "12",
            "--json",
        ])
        .expect("ss should parse as the sessions command");

        let AgentCommand::Sessions(args) = cli.args.command else {
            panic!("ss did not resolve to sessions");
        };
        assert_eq!(args.provider.as_deref(), Some("codex"));
        assert_eq!(args.limit, Some(12));
        assert!(args.json);
    }
    #[test]
    fn ag_is_equivalent_to_agent() {
        let cli = Cli::try_parse_from(["mena", "ag", "claude", "-n"])
            .expect("ag should parse as the agent command");

        let AgentCommand::Agent(args) = cli.args.command else {
            panic!("ag did not resolve to agent");
        };
        assert_eq!(args.provider.as_deref(), Some("claude"));
        assert!(args.fresh);
    }

    #[test]
    fn ps_parses_json_and_verbose_output() {
        let cli =
            Cli::try_parse_from(["mena", "ps", "--json", "--verbose"]).expect("ps should parse");

        let AgentCommand::Ps(args) = cli.args.command else {
            panic!("ps did not resolve to process listing");
        };
        assert!(args.json);
        assert!(args.verbose);
    }

    #[test]
    fn sk_is_equivalent_to_skills() {
        let cli = Cli::try_parse_from([
            "mena",
            "sk",
            "--provider",
            "claude",
            "--scope",
            "global",
            "--json",
        ])
        .expect("sk should parse as the skills command");

        let AgentCommand::Skills(args) = cli.args.command else {
            panic!("sk did not resolve to skills");
        };
        assert_eq!(args.provider.as_deref(), Some("claude"));
        assert_eq!(args.scope.as_deref(), Some("global"));
        assert!(args.json);
    }

    #[test]
    fn mcp_inspect_supports_explicit_live_metadata_discovery() {
        let cli = Cli::try_parse_from([
            "mena",
            "mcp",
            "--provider",
            "codex",
            "--scope",
            "project",
            "inspect",
            "docs",
            "--probe",
            "--timeout",
            "20",
            "--json",
        ])
        .expect("MCP inspection should parse");

        let AgentCommand::Mcp(args) = cli.args.command else {
            panic!("mcp did not resolve to the MCP catalog command");
        };
        assert_eq!(args.provider.as_deref(), Some("codex"));
        assert_eq!(args.scope.as_deref(), Some("project"));
        let Some(mena::McpSubcommand::Inspect {
            name,
            probe,
            timeout,
            json,
        }) = args.command
        else {
            panic!("inspect subcommand was not parsed");
        };
        assert_eq!(name, "docs");
        assert!(probe);
        assert_eq!(timeout, 20);
        assert!(json);
    }
}
