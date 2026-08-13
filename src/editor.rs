use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Open an existing file in the user's configured editor without invoking a shell.
pub fn open_file(path: &Path) -> Result<()> {
    let path = path
        .canonicalize()
        .with_context(|| format!("failed to resolve file {}", path.display()))?;
    if !path.is_file() {
        bail!("path is not a regular file: {}", path.display());
    }

    for variable in ["VISUAL", "EDITOR"] {
        let Some(configured) = std::env::var_os(variable) else {
            continue;
        };
        let configured = configured.to_string_lossy();
        let parts = shlex::split(&configured)
            .with_context(|| format!("{variable} contains invalid shell-style quoting"))?;
        let Some((program, arguments)) = parts.split_first() else {
            continue;
        };
        return run_editor(program, arguments, &path)
            .with_context(|| format!("failed to open {} using {variable}", path.display()));
    }

    for program in ["code", "cursor"] {
        match Command::new(program).arg("--wait").arg(&path).status() {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => bail!("{program} exited with status {status}"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to start {program} for {}", path.display()));
            }
        }
    }

    #[cfg(target_os = "macos")]
    return command_status(&mut Command::new("open"), &path);

    #[cfg(target_os = "windows")]
    return command_status(&mut Command::new("explorer.exe"), &path);

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return command_status(&mut Command::new("xdg-open"), &path);
}

fn run_editor(program: &str, arguments: &[String], path: &Path) -> Result<()> {
    let mut command = Command::new(program);
    command.args(arguments).arg(path);
    let status = command
        .status()
        .with_context(|| format!("failed to start editor `{program}`"))?;
    if !status.success() {
        bail!("editor `{program}` exited with status {status}");
    }
    Ok(())
}

fn command_status(command: &mut Command, path: &Path) -> Result<()> {
    let program = command.get_program().to_string_lossy().into_owned();
    let status = command
        .arg(path)
        .status()
        .with_context(|| format!("failed to start `{program}`"))?;
    if !status.success() {
        bail!("`{program}` exited with status {status}");
    }
    Ok(())
}
