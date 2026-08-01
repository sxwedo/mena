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
mod tests {
    use std::{fs, path::Path};

    use super::{atomic_create_private, atomic_write, parent_or_current};

    #[test]
    fn atomically_creates_and_replaces_a_file() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let path = directory.path().join("output.txt");

        atomic_write(&path, b"first").expect("file should be created");
        assert_eq!(fs::read(&path).expect("file should be readable"), b"first");

        atomic_write(&path, b"second").expect("file should be replaced");
        assert_eq!(fs::read(&path).expect("file should be readable"), b"second");
        assert_eq!(
            fs::read_dir(directory.path())
                .expect("directory should be readable")
                .count(),
            1,
            "temporary files must not remain"
        );
    }

    #[test]
    fn bare_output_names_use_the_current_directory() {
        assert_eq!(parent_or_current(Path::new("output.md")), Path::new("."));
        assert_eq!(
            parent_or_current(Path::new("nested/output.md")),
            Path::new("nested")
        );

        let placeholder =
            tempfile::NamedTempFile::new_in(".").expect("placeholder should be created");
        let bare_path: std::path::PathBuf = placeholder
            .path()
            .file_name()
            .expect("placeholder should have a bare file name")
            .into();
        placeholder
            .close()
            .expect("placeholder should be removed before the write");

        atomic_write(&bare_path, b"bare").expect("bare output should be written");
        assert_eq!(
            fs::read(&bare_path).expect("bare output should be readable"),
            b"bare"
        );
        fs::remove_file(bare_path).expect("bare output should be removable");
    }

    #[test]
    fn privately_creates_without_overwriting_or_leaving_temporary_files() {
        let directory = tempfile::tempdir().expect("temporary directory should be created");
        let path = directory.path().join("private.md");

        assert!(atomic_create_private(&path, b"first").expect("first create"));
        assert!(!atomic_create_private(&path, b"second").expect("collision"));
        assert_eq!(
            fs::read(&path).expect("private file should be readable"),
            b"first"
        );
        assert_eq!(
            fs::read_dir(directory.path())
                .expect("directory should be readable")
                .count(),
            1,
            "temporary files must not remain"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            assert_eq!(
                fs::metadata(&path).expect("metadata").permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
