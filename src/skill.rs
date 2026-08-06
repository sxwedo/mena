use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

mod adapter;

pub use adapter::{discover_skills, parse_skill_detail};

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

/// A catalog of discovered skills across all providers and scopes.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SkillCatalog {
    pub skills: Vec<AgentSkill>,
}

impl SkillCatalog {
    /// Discover all skills in standard locations.
    ///
    /// # Errors
    ///
    /// Returns an error if cataloging fails.
    pub fn scan(home_dir: Option<&Path>, workspace_dir: Option<&Path>) -> Result<Self> {
        let files = discover_skills(home_dir, workspace_dir);
        let mut skills = Vec::with_capacity(files.len());

        for file in files {
            let (description, triggers, valid) = match parse_skill_detail(&file.path) {
                Ok(detail) => (
                    detail.frontmatter.description.or(detail.frontmatter.name),
                    detail.frontmatter.triggers,
                    true,
                ),
                Err(_) => (None, Vec::new(), false),
            };

            skills.push(AgentSkill {
                name: file.name,
                provider: file.provider.as_str().to_string(),
                scope: file.scope.as_str().to_string(),
                path: file.path,
                location: file.location,
                is_symlink: file.is_symlink,
                description,
                triggers,
                valid,
                children: file
                    .children
                    .into_iter()
                    .map(|c| SkillChildItem {
                        name: c.name,
                        path: c.path,
                        is_dir: c.is_dir,
                    })
                    .collect(),
            });
        }

        Ok(Self { skills })
    }

    /// Filter catalog by optional provider and scope strings.
    #[must_use]
    pub fn filter(&self, provider: Option<&str>, scope: Option<&str>) -> Vec<AgentSkill> {
        self.skills
            .iter()
            .filter(|s| {
                if provider.is_some_and(|p| !s.provider.eq_ignore_ascii_case(p)) {
                    return false;
                }
                if scope.is_some_and(|sc| !s.scope.eq_ignore_ascii_case(sc)) {
                    return false;
                }
                true
            })
            .cloned()
            .collect()
    }

    /// Inspect a skill by name (case-insensitive search).
    #[must_use]
    pub fn inspect(&self, name: &str) -> Option<Result<SkillDetail>> {
        let skill = self
            .skills
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))?;

        let raw = match parse_skill_detail(&skill.path) {
            Ok(r) => r,
            Err(e) => return Some(Err(e)),
        };

        Some(Ok(SkillDetail {
            skill: skill.clone(),
            content: raw.content,
            extra: raw.frontmatter.extra,
        }))
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
        assert_eq!(catalog.skills.len(), 1);
        assert_eq!(catalog.skills[0].name, "test-skill");
        assert_eq!(
            catalog.skills[0].description.as_deref(),
            Some("A test skill")
        );

        let filtered = catalog.filter(Some("claude"), Some("global"));
        assert_eq!(filtered.len(), 1);

        let non_matching = catalog.filter(Some("codex"), None);
        assert_eq!(non_matching.len(), 0);

        let detail = catalog.inspect("test-skill").unwrap()?;
        assert_eq!(detail.skill.name, "test-skill");
        assert!(detail.content.contains("Body"));

        Ok(())
    }
}
