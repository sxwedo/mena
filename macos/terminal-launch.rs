//! A private, single-use native executable opened as a document by macOS terminals.
//! Provider argv is decoded as data and exec'd directly; no shell evaluates it.

#[cfg(unix)]
fn run() -> anyhow::Result<()> {
    use anyhow::{Context, ensure};
    use std::io::Read;
    use std::os::unix::{fs::MetadataExt, process::CommandExt};

    let executable = std::env::current_exe()?;
    let directory = executable.parent().context("missing launcher directory")?;
    ensure!(
        executable
            .file_name()
            .is_some_and(|name| name == "Resume.tool")
            && directory
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("mena-resume-")),
        "launcher must run from its private ticket directory"
    );
    let path = directory.join("launch.json");
    let metadata = std::fs::symlink_metadata(&path)?;
    ensure!(
        metadata.is_file()
            && !metadata.file_type().is_symlink()
            && metadata.len() <= 128 * 1024
            && metadata.mode() & 0o777 == 0o600
            && metadata.uid() == std::fs::metadata(&executable)?.uid(),
        "invalid private launch request"
    );
    let mut bytes = Vec::new();
    std::fs::File::open(&path)?
        .take(128 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 128 * 1024, "oversized launch request");
    let request: Request = serde_json::from_slice(&bytes)?;
    ensure!(
        std::path::Path::new(&request.executable).is_absolute()
            && std::path::Path::new(&request.directory).is_absolute(),
        "launch paths must be absolute"
    );
    // Consume the ticket before exec, so it cannot replay a session launch.
    std::fs::remove_file(path)?;
    let _ = std::fs::remove_file(&executable);
    let _ = std::fs::remove_dir(directory);
    let error = std::process::Command::new(request.executable)
        .args(request.arguments)
        .current_dir(request.directory)
        .envs(request.environment)
        .exec();
    Err(error.into())
}

#[derive(serde::Deserialize)]
#[cfg(unix)]
struct Request {
    executable: String,
    arguments: Vec<String>,
    directory: String,
    #[serde(default)]
    environment: std::collections::BTreeMap<String, String>,
}

fn main() {
    #[cfg(unix)]
    if let Err(error) = run() {
        eprintln!("mena: cannot resume session: {error:#}");
        std::process::exit(1);
    }
}
