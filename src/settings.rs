//! Mena configuration loaded from `~/.config/mena/config.toml`.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

const DEFAULT_CONFIG_TEMPLATE: &str = "\
# mena configuration (~/.config/mena/config.toml)
#
# Custom agents use exact executable names plus optional argv markers.
# Resume is native argv, never a shell command. Every resume definition must
# contain a {session} placeholder.
#
# [agent.custom.my_agent]
# executables = [\"my-agent\", \"my-agent.exe\"]
# command_contains = [\"--agent-mode\"]
# resume = [\"my-agent\", \"resume\", \"{session}\"]
";

/// Top-level mena configuration.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct Settings {
    /// Custom developer-agent configuration.
    #[serde(default)]
    pub agent: AgentSettings,
}

/// `[agent]` configuration section.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct AgentSettings {
    /// Named custom process recognizers and optional resume commands.
    #[serde(default)]
    pub custom: BTreeMap<String, CustomAgentSettings>,
}

/// One custom developer-agent process recognizer.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CustomAgentSettings {
    /// Exact executable basenames, for example `my-agent`.
    #[serde(default)]
    pub executables: Vec<String>,
    /// Substrings that must all occur in the process argv.
    #[serde(default)]
    pub command_contains: Vec<String>,
    /// Native argv used to resume; `{session}` is replaced without invoking a shell.
    #[serde(default)]
    pub resume: Vec<String>,
}

impl Settings {
    /// Load settings, returning defaults when the config file is absent.
    ///
    /// # Errors
    ///
    /// Returns an error when the file exists but cannot be read or parsed.
    pub fn load() -> Result<Self> {
        let path = config_path();
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => {
                return Err(error).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        parse_settings(&contents, &path)
    }
}

/// Resolve the mena config directory (`~/.config/mena`).
#[must_use]
pub fn config_dir() -> PathBuf {
    config_base_dir().join("mena")
}

/// Resolve the mena config file (`~/.config/mena/config.toml`).
#[must_use]
pub fn config_path() -> PathBuf {
    config_dir().join("config.toml")
}

/// Create the default config, optionally importing custom agents from clix.
///
/// # Errors
///
/// Returns an error if the destination exists, the legacy config is unavailable
/// or invalid, or the new private config cannot be written.
pub fn ensure_default_config(import_clix: bool) -> Result<PathBuf> {
    let path = config_path();
    if path.exists() {
        bail!("config file already exists: {}", path.display());
    }
    let contents = if import_clix {
        render_imported_config()?
    } else {
        DEFAULT_CONFIG_TEMPLATE.to_owned()
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    write_private(&path, &contents)?;
    Ok(path)
}

fn config_base_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn legacy_clix_config_path() -> PathBuf {
    config_base_dir().join("clix/config.toml")
}

fn render_imported_config() -> Result<String> {
    let path = legacy_clix_config_path();
    let contents = fs::read_to_string(&path)
        .with_context(|| format!("failed to read legacy clix config {}", path.display()))?;
    render_imported_config_from(&contents, &path)
}

fn render_imported_config_from(contents: &str, path: &Path) -> Result<String> {
    let settings = parse_settings(contents, path)?;
    if settings.agent.custom.is_empty() {
        bail!(
            "legacy clix config contains no [agent.custom] entries: {}",
            path.display()
        );
    }
    let serialized = toml::to_string_pretty(&settings)
        .context("failed to serialize imported custom-agent settings")?;
    Ok(format!(
        "# Imported from {} by `mena config init --import-clix`.\n\n{serialized}",
        path.display()
    ))
}

fn parse_settings(contents: &str, path: &Path) -> Result<Settings> {
    toml::from_str(contents).with_context(|| format!("failed to parse {}", path.display()))
}

#[cfg(unix)]
fn write_private(path: &Path, contents: &str) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    file.write_all(contents.as_bytes())
        .with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(not(unix))]
fn write_private(path: &Path, contents: &str) -> Result<()> {
    fs::write(path, contents).with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{DEFAULT_CONFIG_TEMPLATE, Settings, render_imported_config_from};

    #[test]
    fn default_template_is_valid_and_empty() {
        let settings: Settings =
            toml::from_str(DEFAULT_CONFIG_TEMPLATE).expect("default config should parse");
        assert!(settings.agent.custom.is_empty());
    }

    #[test]
    fn custom_agent_config_round_trips() {
        let source = r#"
[agent.custom.my_agent]
executables = ["my-agent", "my-agent.exe"]
command_contains = ["--agent-mode"]
resume = ["my-agent", "resume", "{session}"]
"#;
        let settings: Settings = toml::from_str(source).expect("custom settings should parse");
        let serialized = toml::to_string_pretty(&settings).expect("settings should serialize");
        let reparsed: Settings = toml::from_str(&serialized).expect("serialized settings parse");
        let custom = &reparsed.agent.custom["my_agent"];
        assert_eq!(custom.executables, ["my-agent", "my-agent.exe"]);
        assert_eq!(custom.command_contains, ["--agent-mode"]);
        assert_eq!(custom.resume, ["my-agent", "resume", "{session}"]);
    }

    #[test]
    fn clix_import_keeps_only_custom_agents() {
        let source = r#"
[github]
token = "must-not-be-copied"

[agent.custom.my_agent]
executables = ["my-agent"]
resume = ["my-agent", "resume", "{session}"]
"#;
        let imported = render_imported_config_from(source, Path::new("clix/config.toml"))
            .expect("legacy custom agents should import");
        assert!(imported.contains("[agent.custom.my_agent]"));
        assert!(imported.contains("executables = [\"my-agent\"]"));
        assert!(!imported.contains("must-not-be-copied"));
        assert!(!imported.contains("[github]"));
    }

    #[test]
    fn clix_import_requires_a_custom_agent() {
        let error = render_imported_config_from(
            "[github]\ntoken = \"ignored\"\n",
            Path::new("clix/config.toml"),
        )
        .expect_err("empty legacy agent config should fail");
        assert!(
            error
                .to_string()
                .contains("contains no [agent.custom] entries")
        );
    }
}
