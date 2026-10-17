
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

    let placeholder = tempfile::NamedTempFile::new_in(".").expect("placeholder should be created");
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
