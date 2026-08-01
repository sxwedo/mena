//! Small terminal-output helpers shared by the mena binary and controller.

use std::fmt::Display;
use std::io::{IsTerminal, stderr, stdout};
use std::process::ExitCode;

use anstyle::{AnsiColor, Style};

fn colors_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none()
        && std::env::var_os("TERM").is_none_or(|term| term != "dumb")
}

fn styled(value: &str, color: AnsiColor, stderr_output: bool) -> String {
    let terminal = if stderr_output {
        stderr().is_terminal()
    } else {
        stdout().is_terminal()
    };
    if colors_enabled() && terminal {
        let style = Style::new().fg_color(Some(anstyle::Color::Ansi(color)));
        format!("{style}{value}{style:#}")
    } else {
        value.to_owned()
    }
}

/// Print a neutral status message.
pub fn info(message: impl Display) {
    println!("  {} {message}", styled("◇", AnsiColor::BrightBlack, false));
}

/// Print a successful operation message.
pub fn success(message: impl Display) {
    println!("  {} {message}", styled("✓", AnsiColor::Green, false));
}

/// Convert a result into a conventional process exit code.
#[must_use]
pub fn exit_code(result: anyhow::Result<()>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("  {} {error:#}", styled("✗", AnsiColor::Red, true));
            ExitCode::FAILURE
        }
    }
}
