use std::path::Path;

use anyhow::{Context, Result, bail};
use toml_edit::{Array, DocumentMut, Item, Value, value};

use super::storage;
use crate::mcp::{McpConfigPatch, McpRegistration, McpSourceFormat};

pub(super) fn update_basic_config(
    registration: &McpRegistration,
    patch: &McpConfigPatch,
    workspace: Option<&Path>,
) -> Result<()> {
    ensure_basic_config_editable(registration)?;
    validate_patch(registration, patch)?;
    match (registration.provider.as_str(), registration.source_format) {
        ("codex", McpSourceFormat::Toml) => update_codex_toml(registration, patch),
        (_, McpSourceFormat::Json) => update_standard_json(registration, patch, workspace),
        _ => unreachable!("editability gate accepts only implemented source formats"),
    }
}

pub(super) fn ensure_basic_config_editable(registration: &McpRegistration) -> Result<()> {
    if matches!(registration.scope.as_str(), "plugin" | "managed") {
        bail!(
            "MCP registration `{}` comes from a read-only {} source; open {} to edit it with its owner",
            registration.selector,
            registration.scope,
            registration.source.display()
        );
    }
    match (registration.provider.as_str(), registration.source_format) {
        ("codex", McpSourceFormat::Toml) | (_, McpSourceFormat::Json) => Ok(()),
        (_, format) => bail!(
            "interactive MCP editing does not support {} {} sources; press `o` to open {}",
            registration.provider,
            source_format_name(format),
            registration.source.display()
        ),
    }
}

pub(super) fn basic_config_can_toggle_enabled(registration: &McpRegistration) -> bool {
    matches!(
        registration.provider.as_str(),
        "codex" | "gemini" | "omp" | "opencode"
    )
}

fn update_standard_json(
    registration: &McpRegistration,
    patch: &McpConfigPatch,
    workspace: Option<&Path>,
) -> Result<()> {
    let source_path = editable_source_path(&registration.source)?;
    let content = storage::read_optional_config(&source_path)?.with_context(|| {
        format!(
            "MCP config disappeared before editing: {}",
            source_path.display()
        )
    })?;
    let mut root: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse MCP JSON config {}", source_path.display()))?;
    if registration.provider == "gemini"
        && let Some(enabled) = patch.enabled
    {
        update_gemini_enabled(&mut root, &registration.name, enabled, &source_path)?;
    }
    if registration.provider == "omp"
        && matches!(registration.scope.as_str(), "user" | "profile")
        && let Some(enabled) = patch.enabled
    {
        update_omp_enabled(&mut root, &registration.name, enabled, &source_path)?;
    }
    let opencode_v2 = registration.provider == "opencode"
        && root
            .get("mcp")
            .and_then(|mcp| mcp.get("servers"))
            .is_some_and(serde_json::Value::is_object);
    let server = json_server_mut(&mut root, registration, &source_path, workspace)?;

    if !matches!(
        registration.provider.as_str(),
        "gemini" | "omp" | "opencode"
    ) && let Some(enabled) = patch.enabled
    {
        if server.contains_key("disabled") && !server.contains_key("enabled") {
            server.insert("disabled".to_owned(), serde_json::Value::Bool(!enabled));
        } else {
            server.insert("enabled".to_owned(), serde_json::Value::Bool(enabled));
        }
    }
    if registration.provider == "omp"
        && registration.scope == "project"
        && let Some(enabled) = patch.enabled
    {
        server.insert("enabled".to_owned(), serde_json::Value::Bool(enabled));
    }
    if registration.provider == "opencode"
        && let Some(enabled) = patch.enabled
    {
        if opencode_v2 || server.contains_key("disabled") {
            server.insert("disabled".to_owned(), serde_json::Value::Bool(!enabled));
        } else {
            server.insert("enabled".to_owned(), serde_json::Value::Bool(enabled));
        }
    }
    if registration.provider == "opencode" {
        apply_opencode_command(server, patch);
    } else {
        apply_json_optional_string(server, "command", patch.command.as_ref());
        if let Some(args) = &patch.args {
            server.insert(
                "args".to_owned(),
                serde_json::Value::Array(
                    args.iter()
                        .map(|argument| serde_json::Value::String(argument.clone()))
                        .collect(),
                ),
            );
        }
    }
    apply_json_optional_path(server, "cwd", patch.cwd.as_ref());
    if let Some(url) = patch.url.as_ref() {
        let key = if server.contains_key("httpUrl") {
            "httpUrl"
        } else {
            "url"
        };
        apply_json_optional_string(server, key, Some(url));
    }

    let mut output = serde_json::to_string_pretty(&root)
        .context("failed to serialize edited MCP JSON configuration")?;
    output.push('\n');
    serde_json::from_str::<serde_json::Value>(&output).with_context(|| {
        format!(
            "edited MCP configuration is invalid JSON: {}",
            source_path.display()
        )
    })?;
    storage::check_config_size(&source_path, output.len())?;
    crate::fs::atomic_write(&source_path, output.as_bytes())
}

fn update_omp_enabled(
    root: &mut serde_json::Value,
    name: &str,
    enabled: bool,
    path: &Path,
) -> Result<()> {
    let root = root
        .as_object_mut()
        .with_context(|| format!("MCP config root is not an object: {}", path.display()))?;
    update_name_list(root, "enabledServers", name, enabled, path)?;
    update_name_list(root, "disabledServers", name, !enabled, path)
}

fn update_gemini_enabled(
    root: &mut serde_json::Value,
    name: &str,
    enabled: bool,
    path: &Path,
) -> Result<()> {
    let root = root
        .as_object_mut()
        .with_context(|| format!("MCP config root is not an object: {}", path.display()))?;
    if !root.contains_key("mcp") && enabled {
        return Ok(());
    }
    let policy = root
        .entry("mcp".to_owned())
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .with_context(|| format!("mcp policy is not an object in {}", path.display()))?;
    update_name_list(policy, "excluded", name, !enabled, path)?;
    if let Some(allowed) = policy.get_mut("allowed") {
        let allowed = allowed
            .as_array_mut()
            .with_context(|| format!("mcp.allowed is not an array in {}", path.display()))?;
        if enabled
            && !allowed.is_empty()
            && !allowed.iter().any(|entry| entry.as_str() == Some(name))
        {
            allowed.push(serde_json::Value::String(name.to_owned()));
        }
    }
    Ok(())
}

fn update_name_list(
    root: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    name: &str,
    include: bool,
    path: &Path,
) -> Result<()> {
    let value = root
        .entry(key.to_owned())
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    let entries = value
        .as_array_mut()
        .with_context(|| format!("{key} is not an array in {}", path.display()))?;
    entries.retain(|entry| entry.as_str() != Some(name));
    if include {
        entries.push(serde_json::Value::String(name.to_owned()));
    }
    Ok(())
}

fn json_server_mut<'a>(
    root: &'a mut serde_json::Value,
    registration: &McpRegistration,
    path: &Path,
    workspace: Option<&Path>,
) -> Result<&'a mut serde_json::Map<String, serde_json::Value>> {
    let servers = if registration.provider == "claude" && registration.scope == "local" {
        let workspace = workspace
            .context("cannot edit a Claude local MCP registration without the catalog workspace")?;
        let project_key = nearest_claude_project(root, workspace).with_context(|| {
            format!(
                "no Claude project in {} contains workspace {}",
                path.display(),
                workspace.display()
            )
        })?;
        root.get_mut("projects")
            .and_then(serde_json::Value::as_object_mut)
            .and_then(|projects| projects.get_mut(&project_key))
            .and_then(|project| project.get_mut("mcpServers"))
            .and_then(serde_json::Value::as_object_mut)
            .with_context(|| {
                format!(
                    "Claude project `{project_key}` has no mcpServers object in {}",
                    path.display()
                )
            })?
    } else if registration.provider == "opencode" {
        let mcp = root
            .get_mut("mcp")
            .and_then(serde_json::Value::as_object_mut)
            .with_context(|| format!("{} has no MCP object", path.display()))?;
        if mcp.get("servers").is_some_and(serde_json::Value::is_object) {
            mcp.get_mut("servers")
                .and_then(serde_json::Value::as_object_mut)
                .expect("checked MCP servers object")
        } else {
            mcp
        }
    } else {
        root.get_mut("mcpServers")
            .and_then(serde_json::Value::as_object_mut)
            .with_context(|| format!("{} has no mcpServers object", path.display()))?
    };
    servers
        .get_mut(&registration.name)
        .and_then(serde_json::Value::as_object_mut)
        .with_context(|| {
            format!(
                "MCP registration `{}` is no longer present in {}",
                registration.name,
                path.display()
            )
        })
}

fn nearest_claude_project(root: &serde_json::Value, workspace: &Path) -> Option<String> {
    root.get("projects")
        .and_then(serde_json::Value::as_object)
        .into_iter()
        .flat_map(|projects| projects.keys())
        .filter(|project| path_contains(workspace, Path::new(project)))
        .max_by_key(|project| Path::new(project).components().count())
        .cloned()
}

fn path_contains(path: &Path, ancestor: &Path) -> bool {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let ancestor = ancestor
        .canonicalize()
        .unwrap_or_else(|_| ancestor.to_path_buf());
    path.starts_with(ancestor)
}

fn apply_opencode_command(
    server: &mut serde_json::Map<String, serde_json::Value>,
    patch: &McpConfigPatch,
) {
    if patch.command.is_none() && patch.args.is_none() {
        return;
    }
    let (current_command, current_args) = match server.get("command") {
        Some(serde_json::Value::Array(parts)) => {
            let mut parts = parts.iter().filter_map(serde_json::Value::as_str);
            (
                parts.next().map(str::to_owned),
                parts.map(str::to_owned).collect(),
            )
        }
        Some(serde_json::Value::String(command)) => (
            Some(command.clone()),
            server
                .get("args")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect(),
        ),
        _ => (None, Vec::new()),
    };
    let command = patch.command.clone().unwrap_or(current_command);
    let args = patch.args.clone().unwrap_or(current_args);
    if let Some(command) = command {
        let mut parts = Vec::with_capacity(args.len() + 1);
        parts.push(serde_json::Value::String(command));
        parts.extend(args.into_iter().map(serde_json::Value::String));
        server.insert("command".to_owned(), serde_json::Value::Array(parts));
        server.remove("args");
    }
}

fn update_codex_toml(registration: &McpRegistration, patch: &McpConfigPatch) -> Result<()> {
    let source_path = editable_source_path(&registration.source)?;
    let content = storage::read_optional_config(&source_path)?.with_context(|| {
        format!(
            "MCP config disappeared before editing: {}",
            source_path.display()
        )
    })?;
    let mut document = content
        .parse::<DocumentMut>()
        .with_context(|| format!("failed to parse MCP TOML config {}", source_path.display()))?;
    let servers = document
        .get_mut("mcp_servers")
        .and_then(Item::as_table_like_mut)
        .with_context(|| format!("{} has no [mcp_servers] table", source_path.display()))?;
    let server = servers
        .get_mut(&registration.name)
        .and_then(Item::as_table_like_mut)
        .with_context(|| {
            format!(
                "MCP registration `{}` is no longer present in {}",
                registration.name,
                source_path.display()
            )
        })?;

    if let Some(enabled) = patch.enabled {
        insert_preserving_decor(server, "enabled", value(enabled));
    }
    apply_optional_string(server, "command", patch.command.as_ref());
    if let Some(args) = &patch.args {
        let mut array = Array::new();
        for argument in args {
            array.push(argument.as_str());
        }
        insert_preserving_decor(server, "args", Item::Value(Value::Array(array)));
    }
    apply_optional_path(server, "cwd", patch.cwd.as_ref());
    apply_optional_string(server, "url", patch.url.as_ref());

    let output = document.to_string();
    output.parse::<toml::Value>().with_context(|| {
        format!(
            "edited MCP configuration is invalid TOML: {}",
            source_path.display()
        )
    })?;
    storage::check_config_size(&source_path, output.len())?;
    crate::fs::atomic_write(&source_path, output.as_bytes())
}

fn apply_optional_string(
    table: &mut dyn toml_edit::TableLike,
    key: &str,
    update: Option<&Option<String>>,
) {
    match update {
        Some(Some(value)) => {
            insert_preserving_decor(table, key, toml_edit::value(value.as_str()));
        }
        Some(None) => {
            table.remove(key);
        }
        None => {}
    }
}

fn apply_optional_path(
    table: &mut dyn toml_edit::TableLike,
    key: &str,
    update: Option<&Option<std::path::PathBuf>>,
) {
    match update {
        Some(Some(path)) => {
            insert_preserving_decor(
                table,
                key,
                toml_edit::value(path.to_string_lossy().as_ref()),
            );
        }
        Some(None) => {
            table.remove(key);
        }
        None => {}
    }
}

fn apply_json_optional_string(
    object: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    update: Option<&Option<String>>,
) {
    match update {
        Some(Some(value)) => {
            object.insert(key.to_owned(), serde_json::Value::String(value.clone()));
        }
        Some(None) => {
            object.remove(key);
        }
        None => {}
    }
}

fn apply_json_optional_path(
    object: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    update: Option<&Option<std::path::PathBuf>>,
) {
    match update {
        Some(Some(path)) => {
            object.insert(
                key.to_owned(),
                serde_json::Value::String(path.to_string_lossy().into_owned()),
            );
        }
        Some(None) => {
            object.remove(key);
        }
        None => {}
    }
}

fn validate_patch(registration: &McpRegistration, patch: &McpConfigPatch) -> Result<()> {
    const MAX_FIELD_BYTES: usize = 64 * 1_024;
    const MAX_ARGUMENTS: usize = 1_024;

    if patch.enabled.is_some() && !basic_config_can_toggle_enabled(registration) {
        bail!(
            "{} does not expose a writable per-registration enabled setting",
            registration.provider
        );
    }
    if patch.command == Some(None) {
        bail!("MCP command cannot be removed without changing transport");
    }
    if patch.url == Some(None) {
        bail!("MCP URL cannot be removed without changing transport");
    }
    if let Some(Some(command)) = &patch.command {
        validate_text_field("command", command, MAX_FIELD_BYTES)?;
    }
    if let Some(args) = &patch.args {
        if args.len() > MAX_ARGUMENTS {
            bail!("MCP arguments exceed the {MAX_ARGUMENTS} entry limit");
        }
        for argument in args {
            validate_text_field("argument", argument, MAX_FIELD_BYTES)?;
        }
    }
    if let Some(Some(url)) = &patch.url {
        validate_text_field("URL", url, MAX_FIELD_BYTES)?;
        let parsed = url::Url::parse(url).context("MCP URL must be an absolute HTTP(S) URL")?;
        if !matches!(parsed.scheme(), "http" | "https") {
            bail!("MCP URL must use http or https");
        }
    }
    if patch.command.is_some() && registration.command.is_none() {
        bail!("cannot add a command to a non-stdio MCP registration");
    }
    if patch.url.is_some() && registration.url.is_none() {
        bail!("cannot add a URL to a non-HTTP MCP registration");
    }
    Ok(())
}

fn validate_text_field(name: &str, value: &str, max_bytes: usize) -> Result<()> {
    if value.trim().is_empty() {
        bail!("MCP {name} cannot be empty");
    }
    if value.len() > max_bytes {
        bail!("MCP {name} exceeds the {max_bytes} byte limit");
    }
    if value.contains("<redacted>") {
        bail!(
            "MCP {name} still contains `<redacted>`; replace the placeholder or leave the field unchanged"
        );
    }
    Ok(())
}

fn insert_preserving_decor(table: &mut dyn toml_edit::TableLike, key: &str, mut item: Item) {
    if let Some(previous) = table.get_mut(key) {
        if let (Some(previous), Some(replacement)) = (previous.as_value(), item.as_value_mut()) {
            *replacement.decor_mut() = previous.decor().clone();
        }
        *previous = item;
    } else {
        table.insert(key, item);
    }
}

fn editable_source_path(source: &Path) -> Result<std::path::PathBuf> {
    let path = source
        .canonicalize()
        .with_context(|| format!("failed to resolve MCP config {}", source.display()))?;
    if !path.is_file() {
        bail!("MCP config is not a regular file: {}", path.display());
    }
    Ok(path)
}

const fn source_format_name(format: McpSourceFormat) -> &'static str {
    match format {
        McpSourceFormat::Toml => "TOML",
        McpSourceFormat::Json => "JSON",
        McpSourceFormat::Jsonc => "JSONC",
        McpSourceFormat::Yaml => "YAML",
    }
}
