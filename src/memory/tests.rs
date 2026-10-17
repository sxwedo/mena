
use super::*;
use std::fs;
use tempfile::tempdir;

fn setup(home: &Path, workspace: &Path) -> MemoryCatalog {
    let claude = home.join(".claude");
    fs::create_dir_all(&claude).unwrap();
    fs::write(claude.join("CLAUDE.md"), "# global claude").unwrap();

    let codex = home.join(".codex");
    fs::create_dir_all(&codex).unwrap();
    fs::write(codex.join("AGENTS.md"), "# codex global").unwrap();

    let codex_memories = codex.join("memories");
    fs::create_dir_all(&codex_memories).unwrap();
    fs::write(codex_memories.join("MEMORY.md"), "# codex memory").unwrap();

    fs::write(workspace.join("AGENTS.md"), "# project agents").unwrap();
    fs::write(workspace.join("CLAUDE.md"), "# project claude").unwrap();
    fs::write(workspace.join("CLAUDE.local.md"), "# local").unwrap();

    MemoryCatalog::scan(Some(home), Some(workspace)).unwrap()
}

#[test]
fn catalog_scans_and_filters() -> Result<()> {
    let home = tempdir()?;
    let workspace = tempdir()?;
    let catalog = setup(home.path(), workspace.path());

    assert!(
        catalog
            .files()
            .iter()
            .any(|file| file.provider == "claude" && file.scope == "user")
    );
    assert!(
        catalog
            .files()
            .iter()
            .any(|file| file.name == "memories/MEMORY.md")
    );

    let filtered = catalog.filter(Some("codex"), Some("user"))?;
    assert!(filtered.iter().all(|file| file.provider == "codex"));

    let error = catalog
        .filter(Some("nope"), None)
        .expect_err("unknown provider must be rejected");
    assert!(format!("{error:#}").contains("unsupported memory provider"));
    Ok(())
}

#[test]
fn inspect_reads_content_and_supports_selectors() -> Result<()> {
    let home = tempdir()?;
    let workspace = tempdir()?;
    let catalog = setup(home.path(), workspace.path());

    let detail = catalog.inspect("CLAUDE.md", Some("claude"), Some("user"))?;
    assert!(detail.content.contains("global claude"));

    let detail = catalog.inspect("codex:user:AGENTS.md", None, None)?;
    assert!(detail.content.contains("codex global"));

    Ok(())
}

#[test]
fn ambiguous_names_require_narrowing() -> Result<()> {
    let home = tempdir()?;
    let workspace = tempdir()?;
    let catalog = setup(home.path(), workspace.path());

    let error = catalog
        .inspect("CLAUDE.md", None, None)
        .expect_err("CLAUDE.md exists at both scopes");
    assert!(format!("{error:#}").contains("--provider"));

    Ok(())
}

#[test]
fn delete_removes_cataloged_files_but_rejects_outside_paths() -> Result<()> {
    let home = tempdir()?;
    let workspace = tempdir()?;
    let catalog = setup(home.path(), workspace.path());

    let file = catalog.resolve("CLAUDE.md", Some("claude"), Some("user"))?;
    let removed = catalog.delete(&file.path)?;
    assert!(!removed.exists());

    let outside = tempdir()?.keep().join("notes.md");
    fs::write(&outside, "x")?;
    let error = catalog
        .delete(&outside)
        .expect_err("paths outside provider roots must fail closed");
    assert!(format!("{error:#}").contains("outside provider-owned roots"));
    Ok(())
}

#[test]
fn reads_are_bounded() -> Result<()> {
    let home = tempdir()?;
    let workspace = tempdir()?;
    let catalog = setup(home.path(), workspace.path());

    let file = catalog.resolve("CLAUDE.md", Some("claude"), Some("user"))?;
    fs::write(
        &file.path,
        vec![b'x'; usize::try_from(MAX_MEMORY_BYTES + 1).expect("bound fits in usize")],
    )?;
    let error = catalog
        .read(&file.path)
        .expect_err("oversized memory files must be rejected");
    assert!(format!("{error:#}").contains("refuses to read files over"));
    Ok(())
}
