use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DiscoveredProvider {
    Claude,
    Codex,
    Cursor,
    OpenCode,
    Omp,
    #[allow(dead_code)]
    Generic,
}

impl DiscoveredProvider {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Cursor => "cursor",
            Self::OpenCode => "opencode",
            Self::Omp => "omp",
            Self::Generic => "generic",
        }
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "cursor" => Some(Self::Cursor),
            "opencode" => Some(Self::OpenCode),
            "omp" => Some(Self::Omp),
            "generic" => Some(Self::Generic),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DiscoveredScope {
    Global,
    Workspace,
}

impl DiscoveredScope {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Workspace => "workspace",
        }
    }

    #[must_use]
    #[allow(dead_code)]
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "global" => Some(Self::Global),
            "workspace" => Some(Self::Workspace),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DiscoveredChildItem {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredSkillFile {
    pub name: String,
    pub provider: DiscoveredProvider,
    pub scope: DiscoveredScope,
    pub path: PathBuf,
    pub location: String,
    pub is_symlink: bool,
    pub children: Vec<DiscoveredChildItem>,
}

/// Discover all skill files across standard global and workspace directories.
#[must_use]
pub fn discover_skills(
    home_dir: Option<&Path>,
    workspace_dir: Option<&Path>,
) -> Vec<DiscoveredSkillFile> {
    let mut skills = Vec::new();

    // 1. Discover Global skills
    if let Some(home) = home_dir {
        let global_targets = [
            (
                home.join(".claude").join("skills"),
                DiscoveredProvider::Claude,
            ),
            (home.join(".agents").join("skills"), DiscoveredProvider::Omp),
            (
                home.join(".config").join("opencode").join("skills"),
                DiscoveredProvider::OpenCode,
            ),
            (
                home.join(".codex").join("skills"),
                DiscoveredProvider::Codex,
            ),
            (
                home.join(".cursor").join("rules"),
                DiscoveredProvider::Cursor,
            ),
        ];

        for (root, provider) in global_targets {
            scan_skill_root(
                &root,
                provider,
                DiscoveredScope::Global,
                home_dir,
                &mut skills,
            );
        }
    }

    // 2. Discover Workspace skills
    if let Some(ws) = workspace_dir {
        let workspace_targets = [
            (
                ws.join(".claude").join("skills"),
                DiscoveredProvider::Claude,
            ),
            (ws.join(".agents").join("skills"), DiscoveredProvider::Omp),
            (
                ws.join(".opencode").join("skills"),
                DiscoveredProvider::OpenCode,
            ),
            (ws.join(".codex").join("skills"), DiscoveredProvider::Codex),
            (ws.join(".cursor").join("rules"), DiscoveredProvider::Cursor),
        ];

        for (root, provider) in workspace_targets {
            scan_skill_root(
                &root,
                provider,
                DiscoveredScope::Workspace,
                home_dir,
                &mut skills,
            );
        }
    }

    // Deduplicate or sort by name/scope/provider
    skills.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.scope.cmp(&b.scope))
            .then_with(|| a.provider.cmp(&b.provider))
    });

    skills
}

fn scan_skill_root(
    root: &Path,
    provider: DiscoveredProvider,
    scope: DiscoveredScope,
    home_dir: Option<&Path>,
    out: &mut Vec<DiscoveredSkillFile>,
) {
    if !root.is_dir() {
        return;
    }

    let Ok(entries) = fs::read_dir(root) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            let is_md = path.extension().is_some_and(|ext| ext == "md");
            if is_md && let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                let is_symlink =
                    fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink());
                let location = format_location(root, home_dir);
                out.push(DiscoveredSkillFile {
                    name: stem.to_string(),
                    provider,
                    scope,
                    path: path.clone(),
                    location,
                    is_symlink,
                    children: Vec::new(),
                });
            }
        } else if path.is_dir() {
            // Check for SKILL.md, skill.md, or index.md inside directory
            let skill_file_candidates = [
                path.join("SKILL.md"),
                path.join("skill.md"),
                path.join("README.md"),
            ];

            let found = skill_file_candidates.into_iter().find(|p| p.is_file());
            let dir_name = path.file_name().and_then(|s| s.to_str());

            if let (Some(skill_path), Some(name)) = (found, dir_name) {
                let is_symlink = fs::symlink_metadata(&path)
                    .is_ok_and(|m| m.file_type().is_symlink())
                    || fs::symlink_metadata(&skill_path).is_ok_and(|m| m.file_type().is_symlink());
                let location = format_location(root, home_dir);
                let mut children = Vec::new();

                if let Ok(dir_entries) = fs::read_dir(&path) {
                    for child_entry in dir_entries.flatten() {
                        let c_path = child_entry.path();
                        if let Some(c_name) = c_path.file_name().and_then(|n| n.to_str()) {
                            children.push(DiscoveredChildItem {
                                name: c_name.to_string(),
                                path: c_path.clone(),
                                is_dir: c_path.is_dir(),
                            });
                        }
                    }
                    children
                        .sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
                }

                out.push(DiscoveredSkillFile {
                    name: name.to_string(),
                    provider,
                    scope,
                    path: skill_path,
                    location,
                    is_symlink,
                    children,
                });
            }
        }
    }
}

fn format_location(root: &Path, home_dir: Option<&Path>) -> String {
    let s = root.display().to_string();
    if let Some(home) = home_dir {
        let home_str = home.display().to_string();
        if let Some(stripped) = s.strip_prefix(&home_str) {
            return format!("~{stripped}");
        }
    }
    s
}
#[cfg(test)]
mod tests {
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
        );

        assert_eq!(discovered.len(), 2);
        let names: Vec<_> = discovered.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"ponytail"));
        assert!(names.contains(&"git-flow"));
        Ok(())
    }
}
