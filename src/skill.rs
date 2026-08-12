use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

mod adapter;

use adapter::{
    DiscoveredSkillFile, discover_skills, parse_skill_detail, read_skill_children, read_skill_text,
};

const SKILL_PROVIDERS: &[&str] = &["claude", "codex", "cursor", "opencode", "omp"];
const SKILL_SCOPES: &[&str] = &["global", "workspace"];

/// High-level representation of an Agent Skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSkill {
    pub name: String,
    pub provider: String,
    pub scope: String,
    pub path: PathBuf,
    pub location: String,
    pub is_symlink: bool,
    pub description: Option<String>,
    pub triggers: Vec<String>,
    pub valid: bool,
    pub children: Vec<SkillChildItem>,
}

/// A non-SKILL.md item inside a skill directory (sub-file or sub-directory).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillChildItem {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

/// Detailed inspection view of one Agent Skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillDetail {
    #[serde(flatten)]
    pub skill: AgentSkill,
    pub content: String,
    pub extra: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillEntry {
    pub detail: SkillDetail,
    pub children: Vec<SkillChildItem>,
}

/// A catalog of discovered skills across all providers and scopes.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SkillCatalog {
    skills: Vec<AgentSkill>,
    roots: BTreeMap<PathBuf, PathBuf>,
}

impl SkillCatalog {
    /// Discover all skills in standard locations.
    ///
    /// # Errors
    ///
    /// Returns an error if cataloging fails.
    pub fn scan(home_dir: Option<&Path>, workspace_dir: Option<&Path>) -> Result<Self> {
        let files = discover_skills(home_dir, workspace_dir)?;
        let mut skills = Vec::with_capacity(files.len());
        let mut roots = BTreeMap::new();

        for file in files {
            let DiscoveredSkillFile {
                name,
                provider,
                scope,
                path,
                root,
                location,
                is_symlink,
                children,
            } = file;
            let (description, triggers, valid) = match parse_skill_detail(&path) {
                Ok(detail) => (
                    detail.frontmatter.description.or(detail.frontmatter.name),
                    detail.frontmatter.triggers,
                    true,
                ),
                Err(_) => (None, Vec::new(), false),
            };

            roots.insert(path.clone(), root);

            skills.push(AgentSkill {
                name,
                provider: provider.as_str().to_owned(),
                scope: scope.as_str().to_owned(),
                path,
                location,
                is_symlink,
                description,
                triggers,
                valid,
                children,
            });
        }

        Ok(Self { skills, roots })
    }

    #[must_use]
    pub fn skills(&self) -> &[AgentSkill] {
        &self.skills
    }

    /// Filter catalog by optional provider and scope strings.
    ///
    /// # Errors
    ///
    /// Returns an actionable error when either filter is unsupported.
    pub fn filter(&self, provider: Option<&str>, scope: Option<&str>) -> Result<Vec<AgentSkill>> {
        validate_filter("provider", provider, SKILL_PROVIDERS)?;
        validate_filter("scope", scope, SKILL_SCOPES)?;
        Ok(self.matching(provider, scope).cloned().collect())
    }

    /// Inspect one uniquely identified skill.
    ///
    /// # Errors
    ///
    /// Returns an actionable error when no skill matches, when the name is
    /// ambiguous across providers/scopes, or when its entrypoint cannot be read.
    pub fn inspect(
        &self,
        name: &str,
        provider: Option<&str>,
        scope: Option<&str>,
    ) -> Result<SkillDetail> {
        validate_filter("provider", provider, SKILL_PROVIDERS)?;
        validate_filter("scope", scope, SKILL_SCOPES)?;
        let matches: Vec<_> = self
            .matching(provider, scope)
            .filter(|skill| skill.name.eq_ignore_ascii_case(name))
            .collect();
        let skill = matches
            .first()
            .copied()
            .with_context(|| format!("skill `{name}` not found"))?;
        if matches.len() > 1 {
            let choices = matches
                .iter()
                .map(|skill| format!("{}:{}:{}", skill.provider, skill.scope, skill.name))
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "skill `{name}` is ambiguous ({choices}); narrow it with --provider and/or --scope"
            );
        }
        Ok(self.entry(skill, &skill.path)?.detail)
    }

    pub(crate) fn entry(&self, skill: &AgentSkill, path: &Path) -> Result<SkillEntry> {
        let catalog_skill = self
            .skills
            .iter()
            .find(|candidate| {
                candidate.path == skill.path
                    && candidate.provider == skill.provider
                    && candidate.scope == skill.scope
            })
            .context("refusing to read an entry for a skill outside this catalog")?;
        let root = self
            .roots
            .get(&catalog_skill.path)
            .context("skill catalog is missing its storage root")?;
        validate_entry_path(root, path)?;

        if path.is_dir() {
            let children = read_skill_children(path)?;
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("skill directory")
                .to_owned();
            return Ok(SkillEntry {
                detail: child_detail(catalog_skill, path, name, "[ directory ]".to_owned(), true),
                children,
            });
        }

        if path == catalog_skill.path {
            let raw = parse_skill_detail(path)?;
            return Ok(SkillEntry {
                detail: SkillDetail {
                    skill: catalog_skill.clone(),
                    content: raw.content,
                    extra: raw.frontmatter.extra,
                },
                children: Vec::new(),
            });
        }

        let content = read_skill_text(path)?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("skill file")
            .to_owned();
        Ok(SkillEntry {
            detail: child_detail(catalog_skill, path, name, content, false),
            children: Vec::new(),
        })
    }

    fn matching<'a>(
        &'a self,
        provider: Option<&'a str>,
        scope: Option<&'a str>,
    ) -> impl Iterator<Item = &'a AgentSkill> {
        self.skills.iter().filter(move |skill| {
            provider.is_none_or(|value| skill.provider.eq_ignore_ascii_case(value))
                && scope.is_none_or(|value| skill.scope.eq_ignore_ascii_case(value))
        })
    }
}

fn validate_filter(label: &str, value: Option<&str>, supported: &[&str]) -> Result<()> {
    if let Some(value) = value
        && !supported
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(value))
    {
        bail!(
            "unsupported skill {label} `{value}`; use {}",
            supported.join(", ")
        );
    }
    Ok(())
}

fn validate_entry_path(root: &Path, path: &Path) -> Result<()> {
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("failed to resolve skill root {}", root.display()))?;
    let canonical_path = path
        .canonicalize()
        .with_context(|| format!("failed to resolve skill entry {}", path.display()))?;
    if canonical_path != canonical_root && !canonical_path.starts_with(&canonical_root) {
        bail!(
            "refusing to read skill entry {} outside {}",
            path.display(),
            root.display()
        );
    }
    Ok(())
}

fn child_detail(
    skill: &AgentSkill,
    path: &Path,
    name: String,
    content: String,
    is_directory: bool,
) -> SkillDetail {
    SkillDetail {
        skill: AgentSkill {
            name,
            provider: skill.provider.clone(),
            scope: skill.scope.clone(),
            path: path.to_path_buf(),
            location: skill.location.clone(),
            is_symlink: path
                .symlink_metadata()
                .is_ok_and(|metadata| metadata.file_type().is_symlink()),
            description: is_directory.then_some("Directory".to_owned()),
            triggers: Vec::new(),
            valid: true,
            children: Vec::new(),
        },
        content,
        extra: BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
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
}
