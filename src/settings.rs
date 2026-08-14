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
# border = \"#7ca7d9\"
# popup_title = \"#e1e6eb\"
# metadata_key = \"#7ca7d9\"
# metadata_value = \"#a8b0ba\"
# conversation_header = \"#7ca7d9\"
# empty_text = \"#737d89\"
# status_success = \"#86b98c\"
# status_error = \"#d97b84\"
# footer_key = \"#7ca7d9\"
# footer_text = \"#a8b0ba\"
# footer_separator = \"#343d49\"
# user_header = \"#d3aa6e\"
# user_content = \"#e1e6eb\"
# assistant_header = \"#7ca7d9\"
# assistant_content = \"#e1e6eb\"
# skill_header = \"#a99bcb\"
# skill_content = \"#a8b0ba\"
# tool_call_header = \"#79b8c7\"
# tool_call_content = \"#a8b0ba\"
# tool_result_header = \"#79b8c7\"
# tool_result_content = \"#a8b0ba\"
# system_header = \"#737d89\"
# system_content = \"#737d89\"
# error_header = \"#d97b84\"
# error_content = \"#d97b84\"
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
            border: ConfigColor::Rgb(0x7c, 0xa7, 0xd9),
            popup_title: ConfigColor::Rgb(0xe1, 0xe6, 0xeb),
            metadata_key: ConfigColor::Rgb(0x7c, 0xa7, 0xd9),
            metadata_value: ConfigColor::Rgb(0xa8, 0xb0, 0xba),
            conversation_header: ConfigColor::Rgb(0x7c, 0xa7, 0xd9),
            empty_text: ConfigColor::Rgb(0x73, 0x7d, 0x89),
            status_success: ConfigColor::Rgb(0x86, 0xb9, 0x8c),
            status_error: ConfigColor::Rgb(0xd9, 0x7b, 0x84),
            footer_key: ConfigColor::Rgb(0x7c, 0xa7, 0xd9),
            footer_text: ConfigColor::Rgb(0xa8, 0xb0, 0xba),
            footer_separator: ConfigColor::Rgb(0x34, 0x3d, 0x49),
            user_header: ConfigColor::Rgb(0xd3, 0xaa, 0x6e),
            user_content: ConfigColor::Rgb(0xe1, 0xe6, 0xeb),
            assistant_header: ConfigColor::Rgb(0x7c, 0xa7, 0xd9),
            assistant_content: ConfigColor::Rgb(0xe1, 0xe6, 0xeb),
            skill_header: ConfigColor::Rgb(0xa9, 0x9b, 0xcb),
            skill_content: ConfigColor::Rgb(0xa8, 0xb0, 0xba),
            tool_call_header: ConfigColor::Rgb(0x79, 0xb8, 0xc7),
            tool_call_content: ConfigColor::Rgb(0xa8, 0xb0, 0xba),
            tool_result_header: ConfigColor::Rgb(0x79, 0xb8, 0xc7),
            tool_result_content: ConfigColor::Rgb(0xa8, 0xb0, 0xba),
            system_header: ConfigColor::Rgb(0x73, 0x7d, 0x89),
            system_content: ConfigColor::Rgb(0x73, 0x7d, 0x89),
            error_header: ConfigColor::Rgb(0xd9, 0x7b, 0x84),
            error_content: ConfigColor::Rgb(0xd9, 0x7b, 0x84),
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

/// Create the default configuration file with restrictive permissions.
///
/// # Errors
///
/// Returns an error if the destination exists or the private configuration file
/// cannot be created.
pub fn ensure_default_config() -> Result<PathBuf> {
    let path = config_path();
    if path.exists() {
        bail!("config file already exists: {}", path.display());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    write_private(&path, DEFAULT_CONFIG_TEMPLATE)?;
    Ok(path)
}

fn config_base_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
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
    use super::{ConfigColor, DEFAULT_CONFIG_TEMPLATE, Settings};

    #[test]
    fn default_template_is_valid_and_empty() {
        let settings: Settings =
            toml::from_str(DEFAULT_CONFIG_TEMPLATE).expect("default config should parse");
        assert!(settings.agent.custom.is_empty());
        let colors = settings.ui.session_detail.colors;
        assert_eq!(colors.user_header, ConfigColor::Rgb(0xd3, 0xaa, 0x6e));
        assert_eq!(colors.user_content, ConfigColor::Rgb(0xe1, 0xe6, 0xeb));
        assert_eq!(colors.assistant_header, ConfigColor::Rgb(0x7c, 0xa7, 0xd9));
        assert_eq!(
            colors.tool_result_content,
            ConfigColor::Rgb(0xa8, 0xb0, 0xba)
        );
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
        assert_eq!(colors.tool_call_header, ConfigColor::Rgb(0x79, 0xb8, 0xc7));
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
