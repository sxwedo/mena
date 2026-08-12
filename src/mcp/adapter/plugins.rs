use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use super::DiscoveredMcp;
use super::json_clients::{self, PluginContext};
use super::storage;

const MAX_PLUGINS: usize = 1_000;

pub(super) fn parse(
    home: &Path,
    workspace: Option<&Path>,
    out: &mut Vec<DiscoveredMcp>,
) -> Result<()> {
    parse_claude(home, workspace, out)?;
    parse_codex(home, workspace, out)
}

fn parse_claude(home: &Path, workspace: Option<&Path>, out: &mut Vec<DiscoveredMcp>) -> Result<()> {
    let settings_path = home.join(".claude/settings.json");
    let index_path = home.join(".claude/plugins/installed_plugins.json");
    let Some(settings) = read_json(&settings_path)? else {
        return Ok(());
    };
    let Some(index) = read_json(&index_path)? else {
        return Ok(());
    };
    let enabled = settings
        .get("enabledPlugins")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|plugins| plugins.iter())
        .filter(|(_, enabled)| enabled.as_bool() == Some(true));
    let plugins = index.get("plugins").and_then(Value::as_object);
    let cache_root = home.join(".claude/plugins/cache");
    for (position, (plugin_id, _)) in enabled.enumerate() {
        if position >= MAX_PLUGINS {
            bail!("Claude enabled plugin count exceeds the {MAX_PLUGINS} entry limit");
        }
        let Some(records) = plugins
            .and_then(|plugins| plugins.get(plugin_id))
            .and_then(Value::as_array)
        else {
            continue;
        };
        let Some(record) = records.last() else {
            continue;
        };
        let Some(install_path) = record.get("installPath").and_then(Value::as_str) else {
            continue;
        };
        let root = contained_existing_directory(Path::new(install_path), &cache_root)
            .with_context(|| format!("Claude plugin `{plugin_id}` has an unsafe install path"))?;
        let version = record.get("version").and_then(Value::as_str);
        parse_plugin_root(
            &root,
            ".claude-plugin/plugin.json",
            &PluginContext {
                provider: "claude",
                plugin_id,
                version,
                root: &root,
                workspace,
            },
            out,
        )?;
    }
    Ok(())
}

fn parse_codex(home: &Path, workspace: Option<&Path>, out: &mut Vec<DiscoveredMcp>) -> Result<()> {
    let config_path = home.join(".codex/config.toml");
    let Some(content) = storage::read_optional_config(&config_path)? else {
        return Ok(());
    };
    let config: toml::Value = content.parse().with_context(|| {
        format!(
            "failed to parse Codex plugin config {}",
            config_path.display()
        )
    })?;
    let marketplaces = config.get("marketplaces").and_then(toml::Value::as_table);
    let Some(plugins) = config.get("plugins").and_then(toml::Value::as_table) else {
        return Ok(());
    };
    if plugins.len() > MAX_PLUGINS {
        bail!("Codex configured plugin count exceeds the {MAX_PLUGINS} entry limit");
    }
    for (plugin_id, settings) in plugins {
        if settings.get("enabled").and_then(toml::Value::as_bool) != Some(true) {
            continue;
        }
        let Some((plugin_name, marketplace_name)) = plugin_id.rsplit_once('@') else {
            continue;
        };
        let Some(marketplace_source) = marketplaces
            .and_then(|marketplaces| marketplaces.get(marketplace_name))
            .and_then(|marketplace| marketplace.get("source"))
            .and_then(toml::Value::as_str)
        else {
            // App-backed connectors do not necessarily expose a local manifest.
            continue;
        };
        let marketplace_root = PathBuf::from(marketplace_source);
        let root = contained_existing_directory(
            &marketplace_root.join("plugins").join(plugin_name),
            &marketplace_root,
        )
        .with_context(|| format!("Codex plugin `{plugin_id}` has an unsafe install path"))?;
        let manifest = read_json(&root.join(".codex-plugin/plugin.json"))?;
        let version = manifest
            .as_ref()
            .and_then(|manifest| manifest.get("version"))
            .and_then(Value::as_str);
        parse_plugin_root(
            &root,
            ".codex-plugin/plugin.json",
            &PluginContext {
                provider: "codex",
                plugin_id,
                version,
                root: &root,
                workspace,
            },
            out,
        )?;
    }
    Ok(())
}

fn parse_plugin_root(
    root: &Path,
    manifest_relative: &str,
    context: &PluginContext<'_>,
    out: &mut Vec<DiscoveredMcp>,
) -> Result<()> {
    let manifest_path = root.join(manifest_relative);
    let manifest = read_json(&manifest_path)?;
    match manifest
        .as_ref()
        .and_then(|manifest| manifest.get("mcpServers"))
    {
        Some(Value::String(relative)) => {
            let path = contained_existing_file(&root.join(relative), root).with_context(|| {
                format!(
                    "plugin `{}` MCP manifest escapes its plugin root",
                    context.plugin_id
                )
            })?;
            json_clients::parse_plugin_file(&path, context, out)
        }
        Some(Value::Object(_)) => json_clients::parse_plugin_servers(
            manifest
                .as_ref()
                .and_then(|manifest| manifest.get("mcpServers")),
            &manifest_path,
            context,
            out,
        ),
        Some(_) => bail!(
            "plugin `{}` has an invalid mcpServers declaration in {}",
            context.plugin_id,
            manifest_path.display()
        ),
        None => json_clients::parse_plugin_file(&root.join(".mcp.json"), context, out),
    }
}

fn read_json(path: &Path) -> Result<Option<Value>> {
    let Some(content) = storage::read_optional_config(path)? else {
        return Ok(None);
    };
    serde_json::from_str(&content)
        .with_context(|| format!("failed to parse MCP plugin config {}", path.display()))
        .map(Some)
}

fn contained_existing_directory(path: &Path, root: &Path) -> Result<PathBuf> {
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("failed to resolve plugin root {}", root.display()))?;
    let canonical_path = path
        .canonicalize()
        .with_context(|| format!("failed to resolve plugin path {}", path.display()))?;
    if !canonical_path.starts_with(&canonical_root) || !canonical_path.is_dir() {
        bail!(
            "plugin path {} is outside {}",
            path.display(),
            root.display()
        );
    }
    Ok(canonical_path)
}

fn contained_existing_file(path: &Path, root: &Path) -> Result<PathBuf> {
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("failed to resolve plugin root {}", root.display()))?;
    let canonical_path = path
        .canonicalize()
        .with_context(|| format!("failed to resolve plugin file {}", path.display()))?;
    if !canonical_path.starts_with(&canonical_root) || !canonical_path.is_file() {
        bail!(
            "plugin MCP file {} is outside {}",
            path.display(),
            root.display()
        );
    }
    Ok(canonical_path)
}
