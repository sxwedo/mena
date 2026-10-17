//! Move a Claude Code project store when its directory was renamed.
//!
//! Claude keys the store by a sanitized absolute path. A preview names the
//! source, destination, size, and a confirmation token bound to that
//! snapshot. Committing copies the store aside, renames it, and rewrites
//! only top-level `cwd` fields that still equal the old path.

use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};

use super::claude_project_key;
use super::trash::state_dir;

const RECENT_WRITE: Duration = Duration::from_mins(1);

/// What a relocation would change. `token` must be passed back unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelocatePreview {
    pub from_project: PathBuf,
    pub to_project: PathBuf,
    pub source_dir: PathBuf,
    pub destination_dir: PathBuf,
    pub session_files: usize,
    pub bytes: u64,
    pub token: String,
}

/// Describe a Claude project relocation without changing anything.
///
/// # Errors
///
/// Returns an error when the paths are unsafe, encode to the same store,
/// the destination exists, the source contains a symlink, or a file was
/// written in the last minute.
pub fn preview_claude_relocation(home: &Path, from: &Path, to: &Path) -> Result<RelocatePreview> {
    let plan = plan(home, from, to)?;
    Ok(RelocatePreview {
        from_project: plan.from,
        to_project: plan.to,
        source_dir: plan.source,
        destination_dir: plan.destination,
        session_files: plan.files.len(),
        bytes: plan.bytes,
        token: plan.token,
    })
}

/// Relocation after `preview_claude_relocation`. Returns the backup directory.
///
/// # Errors
///
/// Returns an error when `token` does not match a fresh preview, or when the
/// copy, rename, or `cwd` rewrite fails. The backup is left in place.
pub fn commit_claude_relocation(
    home: &Path,
    from: &Path,
    to: &Path,
    token: &str,
) -> Result<PathBuf> {
    let plan = plan(home, from, to)?;
    if plan.token != token {
        bail!("relocation confirmation does not match the current source; preview again");
    }
    let backup = state_dir(home).join("relocations").join(backup_name());
    copy_tree(&plan.source, &backup.join("original"))?;
    if plan.destination.exists() {
        bail!(
            "Claude project store appeared during relocation: {}",
            plan.destination.display()
        );
    }
    fs::rename(&plan.source, &plan.destination).with_context(|| {
        format!(
            "failed to move {} to {}",
            plan.source.display(),
            plan.destination.display()
        )
    })?;
    for relative in &plan.files {
        rewrite_cwd(&plan.destination.join(relative), &plan.from, &plan.to)?;
    }
    Ok(backup)
}

struct Plan {
    from: PathBuf,
    to: PathBuf,
    source: PathBuf,
    destination: PathBuf,
    files: Vec<PathBuf>,
    bytes: u64,
    token: String,
}

struct Walk {
    files: Vec<(PathBuf, u64, SystemTime)>,
    bytes: u64,
    newest: SystemTime,
}

fn plan(home: &Path, from: &Path, to: &Path) -> Result<Plan> {
    if !from.is_absolute() || !to.is_absolute() {
        bail!("Claude relocation paths must be absolute");
    }
    let from_key = claude_project_key(from)
        .with_context(|| format!("cannot encode Claude project path {}", from.display()))?;
    let to_key = claude_project_key(to)
        .with_context(|| format!("cannot encode Claude project path {}", to.display()))?;
    if from_key == to_key {
        bail!(
            "these paths encode to the same Claude project directory `{from_key}`; mena will not guess which project owns it"
        );
    }
    if !to.is_dir() {
        bail!(
            "relocation target is not an existing directory: {}",
            to.display()
        );
    }
    let projects = home.join(".claude/projects");
    let source = projects.join(&from_key);
    let destination = projects.join(&to_key);
    if !source.is_dir() {
        bail!(
            "no Claude project store at {} (encoded from {})",
            source.display(),
            from.display()
        );
    }
    if destination.exists() {
        bail!(
            "Claude project store already exists at {}",
            destination.display()
        );
    }
    let source_meta = fs::symlink_metadata(&source)
        .with_context(|| format!("failed to inspect {}", source.display()))?;
    if source_meta.file_type().is_symlink() {
        bail!(
            "refusing to relocate symlinked Claude store {}",
            source.display()
        );
    }
    let walk = walk_store(&source)?;
    let newest = walk
        .newest
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let token = format!(
        "{}\n{}\n{}\n{}\n{newest}",
        from.display(),
        to.display(),
        walk.files.len(),
        walk.bytes
    );
    Ok(Plan {
        from: from.to_path_buf(),
        to: to.to_path_buf(),
        source,
        destination,
        files: walk.files.into_iter().map(|(path, _, _)| path).collect(),
        bytes: walk.bytes,
        token,
    })
}

fn walk_store(root: &Path) -> Result<Walk> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    let mut bytes = 0_u64;
    let mut newest = UNIX_EPOCH;
    let now = SystemTime::now();
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory)
            .with_context(|| format!("failed to read {}", directory.display()))?;
        for entry in entries {
            let entry = entry
                .with_context(|| format!("failed to read an entry in {}", directory.display()))?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)
                .with_context(|| format!("failed to inspect {}", path.display()))?;
            if metadata.file_type().is_symlink() {
                bail!("refusing to relocate symlinked path {}", path.display());
            }
            if metadata.is_dir() {
                pending.push(path);
                continue;
            }
            if !metadata.is_file() {
                bail!("refusing to relocate non-file {}", path.display());
            }
            let modified = metadata.modified().with_context(|| {
                format!("failed to read the modification time of {}", path.display())
            })?;
            if now.duration_since(modified).unwrap_or(Duration::ZERO) < RECENT_WRITE {
                bail!(
                    "refusing to relocate {}; it was written in the last minute",
                    path.display()
                );
            }
            bytes = bytes.saturating_add(metadata.len());
            if modified > newest {
                newest = modified;
            }
            let relative = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            files.push((relative, metadata.len(), modified));
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(Walk {
        files,
        bytes,
        newest,
    })
}

fn rewrite_cwd(path: &Path, from: &Path, to: &Path) -> Result<()> {
    if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl")
        && path.file_name().and_then(|name| name.to_str()) != Some("sessions-index.json")
    {
        return Ok(());
    }
    let file = File::open(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut rewritten = Vec::new();
    let mut changed = false;
    let mut line = Vec::new();
    loop {
        line.clear();
        let read = reader
            .read_until(b'\n', &mut line)
            .with_context(|| format!("failed to read {}", path.display()))?;
        if read == 0 {
            break;
        }
        let had_newline = line.last() == Some(&b'\n');
        if had_newline {
            line.pop();
        }
        let output = if let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(&line)
            && replace_cwd(&mut value, from, to)
        {
            changed = true;
            let mut encoded = serde_json::to_vec(&value)
                .with_context(|| format!("failed to encode {}", path.display()))?;
            if had_newline {
                encoded.push(b'\n');
            }
            encoded
        } else {
            if had_newline {
                line.push(b'\n');
            }
            line.clone()
        };
        rewritten.extend_from_slice(&output);
    }
    if changed {
        crate::fs::atomic_write(path, &rewritten)?;
    }
    Ok(())
}

fn replace_cwd(value: &mut serde_json::Value, from: &Path, to: &Path) -> bool {
    let Some(object) = value.as_object_mut() else {
        return false;
    };
    let mut changed = false;
    for key in ["cwd", "projectPath"] {
        if object.get(key).and_then(serde_json::Value::as_str)
            == Some(from.to_str().unwrap_or_default())
            && let Some(slot) = object.get_mut(key)
        {
            *slot = serde_json::Value::String(to.display().to_string());
            changed = true;
        }
    }
    changed
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to).with_context(|| format!("failed to create {}", to.display()))?;
    for entry in fs::read_dir(from).with_context(|| format!("failed to read {}", from.display()))? {
        let entry = entry.with_context(|| format!("failed to read {}", from.display()))?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("failed to inspect {}", path.display()))?;
        if metadata.file_type().is_symlink() {
            bail!("refusing to back up symlinked path {}", path.display());
        }
        let dest = to.join(entry.file_name());
        if metadata.is_dir() {
            copy_tree(&path, &dest)?;
        } else {
            fs::copy(&path, &dest).with_context(|| {
                format!("failed to copy {} to {}", path.display(), dest.display())
            })?;
        }
    }
    Ok(())
}

fn backup_name() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
        .to_string()
}

#[cfg(test)]
mod tests;
