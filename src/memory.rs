//! Provider-neutral discovery and safe handling of agent memory files.
//!
//! Memory files are the Markdown instruction and long-term-memory files each
//! coding agent persists natively (for example `CLAUDE.md`, `AGENTS.md`, or
//! `GEMINI.md`). Discovery is purely static: this module never launches a
//! process and only reads regular files inside provider-owned roots.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

const MEMORY_PROVIDERS: &[&str] = &["claude", "codex", "cursor", "gemini"];
const MEMORY_SCOPES: &[&str] = &["user", "project"];

/// Upper bound for a single memory-file read.
pub const MAX_MEMORY_BYTES: u64 = 1024 * 1024;

/// One discovered agent memory file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryFile {
    pub name: String,
    pub provider: String,
    pub scope: String,
    pub path: PathBuf,
    pub location: String,
    pub size_bytes: u64,
}

/// Detailed view of one memory file, including its bounded content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryDetail {
    #[serde(flatten)]
    pub file: MemoryFile,
    pub content: String,
}

/// A catalog of discovered agent memory files across providers and scopes.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MemoryCatalog {
    files: Vec<MemoryFile>,
    roots: Vec<PathBuf>,
}

impl MemoryCatalog {
    /// Discover memory files in standard provider locations.
    ///
    /// # Errors
    ///
    /// Returns an error if the workspace directory cannot be resolved.
    pub fn scan(home_dir: Option<&Path>, workspace_dir: Option<&Path>) -> Result<Self> {
        let mut catalog = Self::default();

        if let Some(home) = home_dir {
            // User scope
            catalog.push_file(
                "claude",
                "user",
                "CLAUDE.md",
                &home.join(".claude/CLAUDE.md"),
            );
            catalog.push_file("codex", "user", "AGENTS.md", &home.join(".codex/AGENTS.md"));
            catalog.push_directory(
                "codex",
                "user",
                "memories",
                &home.join(".codex/memories"),
                &["md"],
            );
            catalog.push_file(
                "gemini",
                "user",
                "GEMINI.md",
                &home.join(".gemini/GEMINI.md"),
            );

            if let Some(workspace) = workspace_dir {
                // Claude auto-memory is stored under the home directory keyed by
                // the encoded workspace path (`/` replaced with `-`).
                let encoded = workspace.to_string_lossy().replace('/', "-");
                let auto_root = home.join(".claude/projects").join(encoded).join("memory");
                catalog.push_directory("claude", "project", "memory", &auto_root, &["md"]);
            }
        }

        if let Some(workspace) = workspace_dir {
            catalog.push_file(
                "claude",
                "project",
                "CLAUDE.md",
                &workspace.join("CLAUDE.md"),
            );
            catalog.push_file(
                "claude",
                "project",
                "CLAUDE.local.md",
                &workspace.join("CLAUDE.local.md"),
            );
            catalog.push_file(
                "codex",
                "project",
                "AGENTS.md",
                &workspace.join("AGENTS.md"),
            );
            catalog.push_directory(
                "cursor",
                "project",
                "rules",
                &workspace.join(".cursor/rules"),
                &["mdc"],
            );
        }

        Ok(catalog)
    }

    #[must_use]
    pub fn files(&self) -> &[MemoryFile] {
        &self.files
    }

    /// Filter the catalog by optional provider and scope strings.
    ///
    /// # Errors
    ///
    /// Returns an actionable error when either filter is unsupported.
    pub fn filter(&self, provider: Option<&str>, scope: Option<&str>) -> Result<Vec<MemoryFile>> {
        validate_filter("provider", provider, MEMORY_PROVIDERS)?;
        validate_filter("scope", scope, MEMORY_SCOPES)?;
        Ok(self.matching(provider, scope).cloned().collect())
    }

    /// Inspect one uniquely identified memory file.
    ///
    /// The name may be a bare file name or a `provider:scope:name` selector.
    ///
    /// # Errors
    ///
    /// Returns an actionable error when no file matches, when the name is
    /// ambiguous, or when the file cannot be read within the size bound.
    pub fn inspect(
        &self,
        name: &str,
        provider: Option<&str>,
        scope: Option<&str>,
    ) -> Result<MemoryDetail> {
        let file = self.resolve(name, provider, scope)?;
        let content = self.read(&file.path)?;
        Ok(MemoryDetail { file, content })
    }

    /// Resolve one uniquely identified memory file without reading it.
    ///
    /// # Errors
    ///
    /// Returns an actionable error when no file matches or the name is
    /// ambiguous across providers/scopes.
    pub fn resolve(
        &self,
        name: &str,
        provider: Option<&str>,
        scope: Option<&str>,
    ) -> Result<MemoryFile> {
        validate_filter("provider", provider, MEMORY_PROVIDERS)?;
        validate_filter("scope", scope, MEMORY_SCOPES)?;
        let (name, name_provider, name_scope) = split_selector(name);
        let provider = name_provider.or_else(|| provider.map(ToString::to_string));
        let scope = name_scope.or_else(|| scope.map(ToString::to_string));
        let matches: Vec<_> = self
            .matching(provider.as_deref(), scope.as_deref())
            .filter(|file| file.name.eq_ignore_ascii_case(&name))
            .collect();
        let file = matches
            .first()
            .copied()
            .with_context(|| format!("memory file `{name}` not found"))?;
        if matches.len() > 1 {
            let choices = matches
                .iter()
                .map(|file| format!("{}:{}:{}", file.provider, file.scope, file.name))
                .collect::<Vec<_>>()
                .join(", ");
            bail!(
                "memory file `{name}` is ambiguous ({choices}); narrow it with --provider and/or --scope"
            );
        }
        Ok(file.clone())
    }

    /// Read one cataloged file after re-validating root containment.
    ///
    /// # Errors
    ///
    /// Fails closed when the path escapes a provider-owned root, is not a
    /// regular file, or exceeds the bounded-read size.
    pub fn read(&self, path: &Path) -> Result<String> {
        self.validate(path)?;
        let metadata = fs::symlink_metadata(path)
            .with_context(|| format!("failed to inspect {}", path.display()))?;
        if metadata.file_type().is_symlink() {
            bail!("refusing to read symlinked memory file {}", path.display());
        }
        if !metadata.is_file() {
            bail!("memory entry {} is not a regular file", path.display());
        }
        if metadata.len() > MAX_MEMORY_BYTES {
            bail!(
                "memory file {} is {} bytes; mena refuses to read files over {} bytes",
                path.display(),
                metadata.len(),
                MAX_MEMORY_BYTES
            );
        }
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))
    }

    /// Delete one cataloged file after re-validating root containment.
    ///
    /// # Errors
    ///
    /// Fails closed when the path escapes a provider-owned root or is not a
    /// regular file; returns the removed path on success.
    pub fn delete(&self, path: &Path) -> Result<PathBuf> {
        self.validate(path)?;
        let canonical = path
            .canonicalize()
            .with_context(|| format!("failed to resolve {}", path.display()))?;
        let metadata = fs::symlink_metadata(&canonical)
            .with_context(|| format!("failed to inspect {}", canonical.display()))?;
        if !metadata.is_file() {
            bail!(
                "refusing to delete {}: not a regular file",
                canonical.display()
            );
        }
        fs::remove_file(&canonical)
            .with_context(|| format!("failed to delete {}", canonical.display()))?;
        Ok(canonical)
    }

    fn validate(&self, path: &Path) -> Result<()> {
        let canonical = path
            .canonicalize()
            .with_context(|| format!("failed to resolve memory file {}", path.display()))?;
        let contained = self.roots.iter().any(|root| {
            root.canonicalize()
                .is_ok_and(|root| canonical.starts_with(root))
        });
        if !contained {
            bail!(
                "refusing to touch memory file {} outside provider-owned roots",
                path.display()
            );
        }
        Ok(())
    }

    fn push_file(&mut self, provider: &str, scope: &str, name: &str, path: &Path) {
        let Some(location) = self.register(path) else {
            return;
        };
        let size_bytes = fs::metadata(path).map_or(0, |metadata| metadata.len());
        self.files.push(MemoryFile {
            name: name.to_owned(),
            provider: provider.to_owned(),
            scope: scope.to_owned(),
            path: path.to_path_buf(),
            location,
            size_bytes,
        });
    }

    fn push_directory(
        &mut self,
        provider: &str,
        scope: &str,
        label: &str,
        directory: &Path,
        extensions: &[&str],
    ) {
        let Ok(entries) = fs::read_dir(directory) else {
            return;
        };
        let mut files: Vec<_> = entries
            .filter_map(std::result::Result::ok)
            .filter(|entry| {
                let path = entry.path();
                path.is_file()
                    && path
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .is_some_and(|ext| {
                            extensions
                                .iter()
                                .any(|known| ext.eq_ignore_ascii_case(known))
                        })
            })
            .map(|entry| entry.path())
            .collect();
        files.sort();
        for path in files {
            let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let name = format!("{label}/{file_name}");
            self.push_file(provider, scope, &name, &path);
        }
    }

    /// Track the provider-owned root for `path` and return its short location.
    fn register(&mut self, path: &Path) -> Option<String> {
        if !path.is_file() {
            return None;
        }
        let root = path
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        if !self.roots.contains(&root) {
            self.roots.push(root);
        }
        Some(
            path.parent()
                .and_then(|parent| parent.file_name())
                .and_then(|name| name.to_str())
                .unwrap_or(".")
                .to_owned(),
        )
    }

    fn matching<'a>(
        &'a self,
        provider: Option<&'a str>,
        scope: Option<&'a str>,
    ) -> impl Iterator<Item = &'a MemoryFile> {
        self.files.iter().filter(move |file| {
            provider.is_none_or(|value| file.provider.eq_ignore_ascii_case(value))
                && scope.is_none_or(|value| file.scope.eq_ignore_ascii_case(value))
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
            "unsupported memory {label} `{value}`; use {}",
            supported.join(", ")
        );
    }
    Ok(())
}

/// Split an optional `provider:scope:name` selector.
fn split_selector(name: &str) -> (String, Option<String>, Option<String>) {
    let parts: Vec<&str> = name.split(':').collect();
    match parts.as_slice() {
        [provider, scope, name] => (
            (*name).to_owned(),
            Some((*provider).to_owned()),
            Some((*scope).to_owned()),
        ),
        _ => (name.to_owned(), None, None),
    }
}

#[cfg(test)]
mod tests {
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
}
