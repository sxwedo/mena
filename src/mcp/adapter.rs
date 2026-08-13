use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;

use super::{McpConfigPatch, McpRegistration};

mod codex;
mod common;
mod edit;
mod goose;
mod json_clients;
mod plugins;
mod storage;

pub(super) struct DiscoveredMcp {
    pub(super) registration: McpRegistration,
    pub(super) connection: McpConnection,
}

/// Raw connection material is kept private and never serialized. Catalog
/// discovery only records it; use is confined to an explicit live probe.
pub(super) enum McpConnection {
    Stdio {
        command: String,
        args: Vec<String>,
        environment: BTreeMap<String, String>,
        environment_sources: BTreeMap<String, String>,
        environment_templates: BTreeMap<String, (String, Vec<String>)>,
        variables: BTreeMap<String, String>,
        unresolved_values: Vec<String>,
        cwd: Option<PathBuf>,
    },
    Http {
        url: String,
        headers: BTreeMap<String, String>,
        environment_headers: BTreeMap<String, String>,
        environment_templates: BTreeMap<String, (String, Vec<String>)>,
        variables: BTreeMap<String, String>,
        bearer_token_environment: Option<String>,
        unresolved_values: Vec<String>,
    },
    Unsupported {
        reason: String,
    },
}

pub(super) fn discover_mcp_servers(
    home: Option<&Path>,
    workspace: Option<&Path>,
) -> Result<Vec<DiscoveredMcp>> {
    let mut discovered = Vec::new();
    if let Some(home) = home {
        codex::parse(&home.join(".codex/config.toml"), "user", &mut discovered)?;
        goose::parse(&home.join(".config/goose/config.yaml"), &mut discovered)?;
        json_clients::parse_home(home, workspace, &mut discovered)?;
        plugins::parse(home, workspace, &mut discovered)?;
        if dirs::home_dir().as_deref() == Some(home) {
            for path in claude_managed_mcp_paths() {
                json_clients::parse_claude_managed(&path, &mut discovered)?;
            }
        }
    }
    if let Some(workspace) = workspace
        && let Some(path) = storage::find_nearest(workspace, ".codex/config.toml", home)
    {
        codex::parse(&path, "project", &mut discovered)?;
    }
    if let Some(workspace) = workspace {
        json_clients::parse_project(home, workspace, &mut discovered)?;
    }
    Ok(discovered)
}

pub(super) fn update_basic_config(
    registration: &McpRegistration,
    patch: &McpConfigPatch,
    workspace: Option<&Path>,
) -> Result<()> {
    edit::update_basic_config(registration, patch, workspace)
}

pub(super) fn source_line(
    registration: &McpRegistration,
    workspace: Option<&Path>,
) -> Result<usize> {
    edit::source_line(registration, workspace)
}

pub(super) fn delete_config(
    registration: &McpRegistration,
    workspace: Option<&Path>,
) -> Result<()> {
    edit::delete_config(registration, workspace)
}

pub(super) fn ensure_source_editable(registration: &McpRegistration) -> Result<()> {
    edit::ensure_source_editable(registration)
}

pub(super) fn ensure_config_deletable(registration: &McpRegistration) -> Result<()> {
    edit::ensure_config_deletable(registration)
}

fn claude_managed_mcp_paths() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        vec![PathBuf::from(
            "/Library/Application Support/ClaudeCode/managed-mcp.json",
        )]
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        vec![PathBuf::from("/etc/claude-code/managed-mcp.json")]
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("ProgramFiles")
            .map(|root| {
                PathBuf::from(root)
                    .join("ClaudeCode")
                    .join("managed-mcp.json")
            })
            .into_iter()
            .collect()
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "linux",
        target_os = "android",
        target_os = "windows"
    )))]
    {
        Vec::new()
    }
}
