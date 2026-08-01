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

    use super::Cli;

    #[test]
    fn command_tree_is_internally_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn documented_commands_parse() {
        for invocation in [
            vec!["mena", "config", "init", "--import-clix"],
            vec!["mena", "ps", "--json"],
            vec!["mena", "top", "--interval", "3", "--iterations", "1"],
            vec!["mena", "inspect", "codex:42"],
            vec!["mena", "logs", "session-id", "-n", "20"],
            vec!["mena", "sessions", "--plain"],
            vec![
                "mena",
                "sessions",
                "--provider",
                "claude",
                "--limit",
                "10",
                "--json",
            ],
            vec!["mena", "stop", "claude:42"],
            vec!["mena", "resume"],
            vec!["mena", "resume", "--list"],
            vec!["mena", "resume", "gemini:session-id"],
        ] {
            Cli::try_parse_from(&invocation)
                .unwrap_or_else(|error| panic!("failed to parse {invocation:?}: {error}"));
        }
    }
}
