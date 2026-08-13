use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Open an existing file at a one-based line when the selected editor supports it.
pub fn open_file_at_line(path: &Path, line: usize) -> Result<()> {
    open_with_editor(path, line, false)
}

/// Edit an existing file at a one-based line, preferring a terminal editor.
pub fn edit_file_at_line(path: &Path, line: usize) -> Result<()> {
    open_with_editor(path, line, true)
}

fn open_with_editor(path: &Path, line: usize, prefer_terminal: bool) -> Result<()> {
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
        return run_editor(program, arguments, &path, line)
            .with_context(|| format!("failed to open {} using {variable}", path.display()));
    }

    let candidates = if prefer_terminal {
        ["nvim", "vim", "vi", "code", "cursor"]
    } else {
        ["code", "cursor", "nvim", "vim", "vi"]
    };
    for program in candidates {
        match run_editor_status(program, &[], &path, line) {
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

fn run_editor(program: &str, arguments: &[String], path: &Path, line: usize) -> Result<()> {
    let status = run_editor_status(program, arguments, path, line)
        .with_context(|| format!("failed to start editor `{program}`"))?;
    if !status.success() {
        bail!("editor `{program}` exited with status {status}");
    }
    Ok(())
}

fn run_editor_status(
    program: &str,
    arguments: &[String],
    path: &Path,
    line: usize,
) -> std::io::Result<std::process::ExitStatus> {
    Command::new(program)
        .args(editor_arguments(program, arguments, path, line))
        .status()
}

fn editor_arguments(
    program: &str,
    arguments: &[String],
    path: &Path,
    line: usize,
) -> Vec<OsString> {
    let name = Path::new(program)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or(program)
        .to_ascii_lowercase();
    let mut argv = arguments.iter().map(OsString::from).collect::<Vec<_>>();
    match name.as_str() {
        "code" | "code-insiders" | "cursor" => {
            argv.push("--wait".into());
            argv.push("--goto".into());
            argv.push(format!("{}:{line}:1", path.display()).into());
        }
        "vim" | "nvim" | "vi" | "view" => {
            argv.push(format!("+{line}").into());
            argv.push(path.as_os_str().to_owned());
        }
        "nano" => {
            argv.push(format!("+{line},1").into());
            argv.push(path.as_os_str().to_owned());
        }
        "emacs" | "emacsclient" => {
            argv.push(format!("+{line}:1").into());
            argv.push(path.as_os_str().to_owned());
        }
        "hx" | "helix" | "zed" | "subl" | "sublime_text" => {
            argv.push(format!("{}:{line}:1", path.display()).into());
        }
        "idea" | "webstorm" | "pycharm" | "rustrover" => {
            argv.push("--line".into());
            argv.push(line.to_string().into());
            argv.push(path.as_os_str().to_owned());
        }
        _ => argv.push(path.as_os_str().to_owned()),
    }
    argv
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

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;

    use super::editor_arguments;

    #[test]
    fn editors_receive_their_native_line_location_syntax() {
        let path = Path::new("/work/config.toml");
        assert_eq!(
            editor_arguments("nvim", &[], path, 42),
            [OsString::from("+42"), OsString::from(path)]
        );
        assert_eq!(
            editor_arguments("code", &[], path, 42),
            [
                OsString::from("--wait"),
                OsString::from("--goto"),
                OsString::from("/work/config.toml:42:1"),
            ]
        );
    }
}
