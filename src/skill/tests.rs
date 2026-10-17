
use super::*;
use std::fs;
use tempfile::tempdir;

#[test]
fn catalog_scans_and_filters() -> Result<()> {
    let dir = tempdir()?;
    let claude_skills = dir.path().join(".claude").join("skills");
    fs::create_dir_all(&claude_skills)?;

    let test_skill = claude_skills.join("test-skill.md");
    fs::write(
        &test_skill,
        "---\ndescription: \"A test skill\"\ntriggers: test\n---\nBody",
    )?;

    let catalog = SkillCatalog::scan(Some(dir.path()), None)?;
    assert_eq!(catalog.skills().len(), 1);
    assert_eq!(catalog.skills()[0].name, "test-skill");
    assert_eq!(
        catalog.skills()[0].description.as_deref(),
        Some("A test skill")
    );

    let filtered = catalog.filter(Some("claude"), Some("global"))?;
    assert_eq!(filtered.len(), 1);

    let non_matching = catalog.filter(Some("codex"), None)?;
    assert_eq!(non_matching.len(), 0);

    let detail = catalog.inspect("test-skill", None, None)?;
    assert_eq!(detail.skill.name, "test-skill");
    assert!(detail.content.contains("Body"));

    Ok(())
}

#[test]
fn inspect_rejects_ambiguous_names_until_filters_make_them_unique() -> Result<()> {
    let home = tempdir()?;
    let workspace = tempdir()?;
    let global = home.path().join(".claude/skills/shared");
    let local = workspace.path().join(".codex/skills/shared");
    fs::create_dir_all(&global)?;
    fs::create_dir_all(&local)?;
    fs::write(global.join("SKILL.md"), "# Global")?;
    fs::write(local.join("SKILL.md"), "# Workspace")?;

    let catalog = SkillCatalog::scan(Some(home.path()), Some(workspace.path()))?;
    let error = catalog
        .inspect("shared", None, None)
        .expect_err("unqualified duplicate name must be ambiguous");
    assert!(format!("{error:#}").contains("--provider"));

    let detail = catalog.inspect("shared", Some("codex"), Some("workspace"))?;
    assert!(detail.content.contains("Workspace"));
    Ok(())
}

#[test]
fn rejects_unknown_filters() -> Result<()> {
    let catalog = SkillCatalog::scan(None, None)?;
    let error = catalog
        .filter(Some("unknown"), None)
        .expect_err("unknown provider must be rejected");
    assert!(format!("{error:#}").contains("unsupported skill provider"));
    Ok(())
}
