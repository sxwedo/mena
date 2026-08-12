use std::fs::File;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, bail};

const MAX_CONFIG_BYTES: u64 = 8 * 1_024 * 1_024;
const MAX_SERVERS_PER_CONFIG: usize = 10_000;
const MAX_PROFILE_DIRECTORIES: usize = 1_000;

pub(super) fn read_optional_config(path: &Path) -> Result<Option<String>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to open MCP config {}", path.display()));
        }
    };
    let mut bytes = Vec::new();
    file.take(MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read MCP config {}", path.display()))?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        bail!(
            "MCP config {} exceeds the {} MiB read limit",
            path.display(),
            MAX_CONFIG_BYTES / 1_024 / 1_024
        );
    }
    String::from_utf8(bytes)
        .with_context(|| format!("MCP config {} is not valid UTF-8", path.display()))
        .map(Some)
}

pub(super) fn check_server_count(path: &Path, count: usize) -> Result<()> {
    if count > MAX_SERVERS_PER_CONFIG {
        bail!(
            "MCP config {} contains {count} servers, exceeding the {MAX_SERVERS_PER_CONFIG} entry limit",
            path.display()
        );
    }
    Ok(())
}

pub(super) fn find_nearest(
    start: &Path,
    relative: impl AsRef<Path>,
    stop_before: Option<&Path>,
) -> Option<std::path::PathBuf> {
    let relative = relative.as_ref();
    start
        .ancestors()
        .take_while(|ancestor| stop_before != Some(*ancestor))
        .map(|ancestor| ancestor.join(relative))
        .find(|candidate| candidate.is_file())
}

pub(super) fn child_directories(path: &Path) -> Result<Vec<std::path::PathBuf>> {
    let entries = match std::fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(error).with_context(|| {
                format!("failed to enumerate MCP profiles in {}", path.display())
            });
        }
    };
    let mut directories = Vec::new();
    for entry in entries.take(MAX_PROFILE_DIRECTORIES + 1) {
        let entry = entry.with_context(|| {
            format!("failed to read an MCP profile entry in {}", path.display())
        })?;
        if entry
            .file_type()
            .with_context(|| format!("failed to inspect MCP profile {}", entry.path().display()))?
            .is_dir()
        {
            directories.push(entry.path());
        }
    }
    if directories.len() > MAX_PROFILE_DIRECTORIES {
        bail!(
            "MCP profile root {} exceeds the {MAX_PROFILE_DIRECTORIES} directory limit",
            path.display()
        );
    }
    directories.sort();
    Ok(directories)
}
