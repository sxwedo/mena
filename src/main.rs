use anyhow::Result;
use clap::Parser;
use mena::AgentArgs;

#[derive(Debug, Parser)]
#[command(
    name = "mena",
    author,
    version,
    about = "Local-first process and session control for developer agents"
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
    fn documented_commands_parse() {
        for invocation in [
            vec!["mena", "config", "init", "--import-clix"],
            vec!["mena", "agent"],
            vec!["mena", "ag"],
            vec!["mena", "ag", "claude", "-n"],
            vec!["mena", "ag", "omp", "--resume"],
            vec!["mena", "ag", "codex", "--session", "session-123"],
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
}
