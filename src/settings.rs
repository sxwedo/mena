//! Mena configuration loaded from `~/.config/mena/config.toml`.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

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
#
# Session detail colors accept ANSI names, ansi:0 through ansi:255, or #RRGGBB.
# Every value below is optional; these commented values document the defaults.
#
# [ui.session_detail.colors]
# border = \"cyan\"
# popup_title = \"reset\"
# metadata_key = \"light-magenta\"
# metadata_value = \"reset\"
# conversation_header = \"cyan\"
# empty_text = \"dark-gray\"
# status_success = \"green\"
# status_error = \"red\"
# footer_key = \"cyan\"
# footer_text = \"reset\"
# footer_separator = \"dark-gray\"
# user_header = \"light-green\"
# user_content = \"light-green\"
# assistant_header = \"cyan\"
# assistant_content = \"cyan\"
# skill_header = \"light-yellow\"
# skill_content = \"light-yellow\"
# tool_call_header = \"dark-gray\"
# tool_call_content = \"dark-gray\"
# tool_result_header = \"dark-gray\"
# tool_result_content = \"dark-gray\"
# system_header = \"dark-gray\"
# system_content = \"dark-gray\"
# error_header = \"dark-gray\"
# error_content = \"dark-gray\"
";

/// Top-level mena configuration.
#[derive(Debug, Default, Clone, Deserialize, Serialize)]
pub struct Settings {
    /// Custom developer-agent configuration.
    #[serde(default)]
    pub agent: AgentSettings,
    /// Terminal interface configuration.
    #[serde(default, skip_serializing_if = "UiSettings::is_default")]
    pub ui: UiSettings,
}

/// `[ui]` terminal interface configuration.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct UiSettings {
    /// Session detail popup configuration.
    #[serde(default)]
    pub session_detail: SessionDetailSettings,
}

impl UiSettings {
    fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

/// `[ui.session_detail]` configuration.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SessionDetailSettings {
    /// Colors used by the session detail popup.
    #[serde(default)]
    pub colors: SessionDetailColorSettings,
}

/// `[ui.session_detail.colors]` configuration.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct SessionDetailColorSettings {
    pub border: ConfigColor,
    pub popup_title: ConfigColor,
    pub metadata_key: ConfigColor,
    pub metadata_value: ConfigColor,
    pub conversation_header: ConfigColor,
    pub empty_text: ConfigColor,
    pub status_success: ConfigColor,
    pub status_error: ConfigColor,
    pub footer_key: ConfigColor,
    pub footer_text: ConfigColor,
    pub footer_separator: ConfigColor,
    pub user_header: ConfigColor,
    pub user_content: ConfigColor,
    pub assistant_header: ConfigColor,
    pub assistant_content: ConfigColor,
    pub skill_header: ConfigColor,
    pub skill_content: ConfigColor,
    pub tool_call_header: ConfigColor,
    pub tool_call_content: ConfigColor,
    pub tool_result_header: ConfigColor,
    pub tool_result_content: ConfigColor,
    pub system_header: ConfigColor,
    pub system_content: ConfigColor,
    pub error_header: ConfigColor,
    pub error_content: ConfigColor,
}

impl Default for SessionDetailColorSettings {
    fn default() -> Self {
        Self {
            border: ConfigColor::Cyan,
            popup_title: ConfigColor::Reset,
            metadata_key: ConfigColor::LightMagenta,
            metadata_value: ConfigColor::Reset,
            conversation_header: ConfigColor::Cyan,
            empty_text: ConfigColor::DarkGray,
            status_success: ConfigColor::Green,
            status_error: ConfigColor::Red,
            footer_key: ConfigColor::Cyan,
            footer_text: ConfigColor::Reset,
            footer_separator: ConfigColor::DarkGray,
            user_header: ConfigColor::LightGreen,
            user_content: ConfigColor::LightGreen,
            assistant_header: ConfigColor::Cyan,
            assistant_content: ConfigColor::Cyan,
            skill_header: ConfigColor::LightYellow,
            skill_content: ConfigColor::LightYellow,
            tool_call_header: ConfigColor::DarkGray,
            tool_call_content: ConfigColor::DarkGray,
            tool_result_header: ConfigColor::DarkGray,
            tool_result_content: ConfigColor::DarkGray,
            system_header: ConfigColor::DarkGray,
            system_content: ConfigColor::DarkGray,
            error_header: ConfigColor::DarkGray,
            error_content: ConfigColor::DarkGray,
        }
    }
}

/// A terminal color accepted by mena configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigColor {
    Reset,
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    Gray,
    DarkGray,
    LightRed,
    LightGreen,
    LightYellow,
    LightBlue,
    LightMagenta,
    LightCyan,
    White,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

impl FromStr for ConfigColor {
    type Err = String;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        let normalized = value.trim().to_ascii_lowercase().replace('_', "-");
        let named = match normalized.as_str() {
            "reset" | "default" => Some(Self::Reset),
            "black" => Some(Self::Black),
            "red" => Some(Self::Red),
            "green" => Some(Self::Green),
            "yellow" => Some(Self::Yellow),
            "blue" => Some(Self::Blue),
            "magenta" => Some(Self::Magenta),
            "cyan" => Some(Self::Cyan),
            "gray" | "grey" => Some(Self::Gray),
            "dark-gray" | "dark-grey" => Some(Self::DarkGray),
            "light-red" => Some(Self::LightRed),
            "light-green" => Some(Self::LightGreen),
            "light-yellow" => Some(Self::LightYellow),
            "light-blue" => Some(Self::LightBlue),
            "light-magenta" => Some(Self::LightMagenta),
            "light-cyan" => Some(Self::LightCyan),
            "white" => Some(Self::White),
            _ => None,
        };
        if let Some(color) = named {
            return Ok(color);
        }
        if let Some(index) = normalized
            .strip_prefix("ansi:")
            .or_else(|| normalized.strip_prefix("indexed:"))
        {
            return index
                .parse::<u8>()
                .map(Self::Indexed)
                .map_err(|_| invalid_color(value));
        }
        if let Some(hex) = normalized.strip_prefix('#')
            && hex.len() == 6
            && hex.is_ascii()
        {
            let red = u8::from_str_radix(&hex[0..2], 16).map_err(|_| invalid_color(value))?;
            let green = u8::from_str_radix(&hex[2..4], 16).map_err(|_| invalid_color(value))?;
            let blue = u8::from_str_radix(&hex[4..6], 16).map_err(|_| invalid_color(value))?;
            return Ok(Self::Rgb(red, green, blue));
        }
        Err(invalid_color(value))
    }
}

fn invalid_color(value: &str) -> String {
    format!("unsupported color `{value}`; use an ANSI name, ansi:0..255, or a #RRGGBB value")
}

impl fmt::Display for ConfigColor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reset => formatter.write_str("reset"),
            Self::Black => formatter.write_str("black"),
            Self::Red => formatter.write_str("red"),
            Self::Green => formatter.write_str("green"),
            Self::Yellow => formatter.write_str("yellow"),
            Self::Blue => formatter.write_str("blue"),
            Self::Magenta => formatter.write_str("magenta"),
            Self::Cyan => formatter.write_str("cyan"),
            Self::Gray => formatter.write_str("gray"),
            Self::DarkGray => formatter.write_str("dark-gray"),
            Self::LightRed => formatter.write_str("light-red"),
            Self::LightGreen => formatter.write_str("light-green"),
            Self::LightYellow => formatter.write_str("light-yellow"),
            Self::LightBlue => formatter.write_str("light-blue"),
            Self::LightMagenta => formatter.write_str("light-magenta"),
            Self::LightCyan => formatter.write_str("light-cyan"),
            Self::White => formatter.write_str("white"),
            Self::Indexed(index) => write!(formatter, "ansi:{index}"),
            Self::Rgb(red, green, blue) => write!(formatter, "#{red:02x}{green:02x}{blue:02x}"),
        }
    }
}

impl Serialize for ConfigColor {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ConfigColor {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(de::Error::custom)
    }
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

    use super::{ConfigColor, DEFAULT_CONFIG_TEMPLATE, Settings, render_imported_config_from};

    #[test]
    fn default_template_is_valid_and_empty() {
        let settings: Settings =
            toml::from_str(DEFAULT_CONFIG_TEMPLATE).expect("default config should parse");
        assert!(settings.agent.custom.is_empty());
        let colors = settings.ui.session_detail.colors;
        assert_eq!(colors.user_header, ConfigColor::LightGreen);
        assert_eq!(colors.user_content, ConfigColor::LightGreen);
        assert_eq!(colors.assistant_header, ConfigColor::Cyan);
        assert_eq!(colors.tool_result_content, ConfigColor::DarkGray);
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
        assert!(!imported.contains("[ui]"));
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

    #[test]
    fn session_detail_colors_accept_names_indexed_colors_and_rgb() {
        let source = r##"
[ui.session_detail.colors]
user_header = "white"
user_content = "#a1B2c3"
assistant_header = "ansi:45"
assistant_content = "indexed:200"
"##;

        let settings: Settings = toml::from_str(source).expect("color settings should parse");
        let colors = settings.ui.session_detail.colors;

        assert_eq!(colors.user_header, ConfigColor::White);
        assert_eq!(colors.user_content, ConfigColor::Rgb(0xa1, 0xb2, 0xc3));
        assert_eq!(colors.assistant_header, ConfigColor::Indexed(45));
        assert_eq!(colors.assistant_content, ConfigColor::Indexed(200));
        assert_eq!(colors.tool_call_header, ConfigColor::DarkGray);
    }

    #[test]
    fn invalid_session_detail_color_fails_with_an_actionable_error() {
        let error = toml::from_str::<Settings>(
            "[ui.session_detail.colors]\nuser_header = \"ultraviolet\"\n",
        )
        .expect_err("unknown colors must fail");

        assert!(
            error
                .to_string()
                .contains("unsupported color `ultraviolet`")
        );
        assert!(error.to_string().contains("#RRGGBB"));
    }
}
