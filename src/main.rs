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
}
