//! Mena-owned display-title overlays for cataloged sessions.
//!
//! Titles live in `session-titles.toml` under the mena config directory (or
//! beside a non-user home used by tests). Native provider session files are
//! never rewritten.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::AgentSession;

const OVERLAY_FILE: &str = "session-titles.toml";
const MAX_TITLE_CHARS: usize = 120;

#[derive(Debug, Default, Serialize, Deserialize)]
struct TitleFile {
    #[serde(default)]
    titles: BTreeMap<String, String>,
}

/// Overlay path for a catalog rooted at `home`.
///
/// The current user's home uses the mena config directory so `XDG_CONFIG_HOME`
/// is honored. Any other root (tests, alternate homes) keeps the overlay under
/// that home so scans cannot read or write the real user file.
pub(super) fn path_for(home: &Path) -> PathBuf {
    if dirs::home_dir().is_some_and(|user_home| super::paths_equivalent(&user_home, home)) {
        crate::settings::config_dir().join(OVERLAY_FILE)
    } else {
        home.join(".config/mena").join(OVERLAY_FILE)
    }
}

pub(super) fn load(path: &Path) -> Result<BTreeMap<String, String>> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) if contents.trim().is_empty() => return Ok(BTreeMap::new()),
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    let parsed: TitleFile =
        toml::from_str(&contents).with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(parsed
        .titles
        .into_iter()
        .filter(|(_, title)| !title.trim().is_empty())
        .collect())
}

pub(super) fn apply(sessions: &mut [AgentSession], overlay: &BTreeMap<String, String>) {
    for session in sessions {
        if let Some(title) = overlay.get(&session.target()) {
            session.title = Some(title.clone());
        }
    }
}

/// Collapse whitespace and enforce the display-title length limit.
///
/// Returns `Ok(None)` when the title is empty after normalization so the
/// caller can drop the overlay and restore the provider-native title.
pub(super) fn normalize(title: &str) -> Result<Option<String>> {
    let normalized = title.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return Ok(None);
    }
    if normalized.chars().count() > MAX_TITLE_CHARS {
        bail!("session title must be at most {MAX_TITLE_CHARS} characters");
    }
    Ok(Some(normalized))
}

pub(super) fn put(path: &Path, target: &str, title: Option<String>) -> Result<()> {
    let mut titles = load(path)?;
    match title {
        Some(title) => {
            titles.insert(target.to_owned(), title);
        }
        None => {
            titles.remove(target);
        }
    }
    persist(path, &titles)
}

fn persist(path: &Path, titles: &BTreeMap<String, String>) -> Result<()> {
    if titles.is_empty() {
        return remove_if_present(path);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let rendered = toml::to_string_pretty(&TitleFile {
        titles: titles.clone(),
    })
    .context("failed to serialize session title overlay")?;
    if path.exists() {
        crate::fs::atomic_write(path, rendered.as_bytes())
    } else if crate::fs::atomic_create_private(path, rendered.as_bytes())? {
        Ok(())
    } else {
        crate::fs::atomic_write(path, rendered.as_bytes())
    }
}

fn remove_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("failed to remove {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_TITLE_CHARS, load, normalize, persist, put};

    #[test]
    fn normalize_collapses_whitespace_and_clears_blank_titles() {
        assert_eq!(
            normalize("  Custom   name  ").expect("valid title"),
            Some("Custom name".to_owned())
        );
        assert_eq!(normalize("   ").expect("blank title"), None);
        assert_eq!(normalize("").expect("empty title"), None);
    }

    #[test]
    fn normalize_rejects_titles_over_the_display_limit() {
        let allowed = "a".repeat(MAX_TITLE_CHARS);
        assert_eq!(
            normalize(&allowed).expect("limit is allowed"),
            Some(allowed)
        );
        let error = normalize(&"a".repeat(MAX_TITLE_CHARS + 1)).expect_err("over limit");
        assert!(format!("{error:#}").contains("at most 120 characters"));
    }

    #[test]
    fn overlay_round_trips_colon_keys_and_drops_the_file_when_empty() {
        let directory = tempfile::tempdir().expect("temporary overlay directory");
        let path = directory.path().join("session-titles.toml");

        put(&path, "codex:abc-123", Some("Refactor catalog".to_owned())).expect("write overlay");
        let loaded = load(&path).expect("read overlay");
        assert_eq!(
            loaded.get("codex:abc-123").map(String::as_str),
            Some("Refactor catalog")
        );

        put(&path, "codex:abc-123", None).expect("clear overlay");
        assert!(!path.exists(), "empty overlay file should be removed");
        assert!(load(&path).expect("missing overlay").is_empty());
    }

    #[test]
    fn persist_replaces_an_existing_overlay_atomically() {
        let directory = tempfile::tempdir().expect("temporary overlay directory");
        let path = directory.path().join("session-titles.toml");
        let mut titles = std::collections::BTreeMap::new();
        titles.insert("claude:one".to_owned(), "First".to_owned());
        persist(&path, &titles).expect("create overlay");
        titles.insert("claude:one".to_owned(), "Second".to_owned());
        persist(&path, &titles).expect("replace overlay");
        let loaded = load(&path).expect("read overlay");
        assert_eq!(loaded.get("claude:one").map(String::as_str), Some("Second"));
    }
}
