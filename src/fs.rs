use std::{fs, io::Write, path::Path};

#[cfg(unix)]
use std::fs::File;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use anyhow::{Context, Result};

/// Atomically replace `path` with `content`.
///
/// The temporary file is created beside the destination so the final rename
/// stays on one filesystem. Existing permissions are retained. The file is
/// synced before success is reported, as is the containing directory on
/// platforms that support syncing directory handles.
///
/// # Errors
///
/// Returns an error if the temporary file cannot be created, written, synced,
/// or persisted as the destination.
pub fn atomic_write(path: &Path, content: &[u8]) -> Result<()> {
    let parent = parent_or_current(path);
    let mut builder = tempfile::Builder::new();
    #[cfg(unix)]
    builder.permissions(fs::Permissions::from_mode(0o666));
    let mut temporary = builder
        .tempfile_in(parent)
        .with_context(|| format!("failed to create a temporary file in {}", parent.display()))?;

    if let Ok(metadata) = fs::metadata(path) {
        temporary
            .as_file()
            .set_permissions(metadata.permissions())
            .with_context(|| format!("failed to preserve permissions for {}", path.display()))?;
    }

    temporary
        .write_all(content)
        .with_context(|| format!("failed to write temporary output for {}", path.display()))?;
    temporary
        .as_file_mut()
        .sync_all()
        .with_context(|| format!("failed to sync temporary output for {}", path.display()))?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("failed to replace {}", path.display()))?;

    sync_directory(parent)
        .with_context(|| format!("failed to sync output directory {}", parent.display()))
}

/// Atomically create a private file without replacing an existing destination.
///
/// Returns `Ok(true)` when the file was created and `Ok(false)` when `path`
/// already existed. On Unix, the created file has mode `0600` regardless of
/// the process umask.
///
/// # Errors
///
/// Returns an error if a temporary file cannot be created, written, synced, or
/// persisted, or if the containing directory cannot be synced.
pub fn atomic_create_private(path: &Path, content: &[u8]) -> Result<bool> {
    let parent = parent_or_current(path);
    let mut builder = tempfile::Builder::new();
    #[cfg(unix)]
    builder.permissions(fs::Permissions::from_mode(0o600));
    let mut temporary = builder
        .tempfile_in(parent)
        .with_context(|| format!("failed to create a temporary file in {}", parent.display()))?;
    temporary
        .write_all(content)
        .with_context(|| format!("failed to write temporary output for {}", path.display()))?;
    temporary
        .as_file_mut()
        .sync_all()
        .with_context(|| format!("failed to sync temporary output for {}", path.display()))?;

    match temporary.persist_noclobber(path) {
        Ok(_) => {
            sync_directory(parent)
                .with_context(|| format!("failed to sync output directory {}", parent.display()))?;
            Ok(true)
        }
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => {
            Err(error.error).with_context(|| format!("failed to create {}", path.display()))
        }
    }
}

/// Return `path`'s parent, treating a bare file name as the current directory.
#[must_use]
pub fn parent_or_current(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests;
