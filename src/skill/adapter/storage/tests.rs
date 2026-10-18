use super::*;
use anyhow::Result;
use tempfile::tempdir;

#[test]
fn scans_skill_directories() -> Result<()> {
    let dir = tempdir()?;
    let claude_skills = dir.path().join(".claude").join("skills");
    fs::create_dir_all(&claude_skills)?;

    let ponytail_dir = claude_skills.join("ponytail");
    fs::create_dir_all(&ponytail_dir)?;
    fs::write(ponytail_dir.join("SKILL.md"), "---\nname: ponytail\n---")?;

    let direct_skill = claude_skills.join("git-flow.md");
    fs::write(direct_skill, "# Git Flow")?;

    let mut discovered = Vec::new();
    scan_skill_root(
        &claude_skills,
        DiscoveredProvider::Claude,
        DiscoveredScope::Global,
        None,
        &mut discovered,
    )?;

    assert_eq!(discovered.len(), 2);
    let names: Vec<_> = discovered.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"ponytail"));
    assert!(names.contains(&"git-flow"));
    Ok(())
}
