use std::path::Path;

use anyhow::{Context, Result, bail};
use toml_edit::{Array, DocumentMut, Item, Key, Value, value};

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

pub(super) fn source_line(
    registration: &McpRegistration,
    workspace: Option<&Path>,
) -> Result<usize> {
    let source_path = editable_source_path(&registration.source)?;
    let content = storage::read_optional_config(&source_path)?.with_context(|| {
        format!(
            "MCP config disappeared before opening: {}",
            source_path.display()
        )
    })?;
    let offset = match registration.source_format {
        McpSourceFormat::Toml => toml_registration_offset(&content, registration),
        McpSourceFormat::Json | McpSourceFormat::Jsonc => {
            json_registration_offset(&content, registration, workspace)
        }
        McpSourceFormat::Yaml => yaml_registration_offset(&content, &registration.name),
    };
    Ok(offset.map_or(1, |offset| {
        content[..offset.min(content.len())]
            .bytes()
            .filter(|byte| *byte == b'\n')
            .count()
            + 1
    }))
}

pub(super) fn delete_config(
    registration: &McpRegistration,
    workspace: Option<&Path>,
) -> Result<()> {
    ensure_config_deletable(registration)?;
    match (registration.provider.as_str(), registration.source_format) {
        ("codex", McpSourceFormat::Toml) => delete_codex_toml(registration),
        (_, McpSourceFormat::Json) => delete_standard_json(registration, workspace),
        _ => unreachable!("delete gate accepts only implemented source formats"),
    }
}

pub(super) fn ensure_source_editable(registration: &McpRegistration) -> Result<()> {
    if matches!(registration.scope.as_str(), "plugin" | "managed") {
        bail!(
            "MCP registration `{}` comes from a read-only {} source; use its owner to edit {}",
            registration.selector,
            registration.scope,
            registration.source.display()
        );
    }
    Ok(())
}

pub(super) fn ensure_config_deletable(registration: &McpRegistration) -> Result<()> {
    ensure_source_editable(registration)?;
    match (registration.provider.as_str(), registration.source_format) {
        ("codex", McpSourceFormat::Toml) | (_, McpSourceFormat::Json) => Ok(()),
        (_, format) => bail!(
            "MCP deletion does not support {} {} sources because their comments cannot be preserved; press `e` to edit {}",
            registration.provider,
            source_format_name(format),
            registration.source.display()
        ),
    }
}

pub(super) fn ensure_basic_config_editable(registration: &McpRegistration) -> Result<()> {
    ensure_source_editable(registration)?;
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

fn delete_standard_json(registration: &McpRegistration, workspace: Option<&Path>) -> Result<()> {
    let source_path = editable_source_path(&registration.source)?;
    let content = storage::read_optional_config(&source_path)?.with_context(|| {
        format!(
            "MCP config disappeared before deletion: {}",
            source_path.display()
        )
    })?;
    let mut root: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse MCP JSON config {}", source_path.display()))?;
    let removed = json_servers_mut(&mut root, registration, &source_path, workspace)?
        .remove(&registration.name)
        .is_some();
    if !removed {
        bail!(
            "MCP registration `{}` is no longer present in {}",
            registration.name,
            source_path.display()
        );
    }
    if registration.provider == "gemini"
        && let Some(policy) = root
            .get_mut("mcp")
            .and_then(serde_json::Value::as_object_mut)
    {
        remove_name_from_list(policy, "allowed", &registration.name);
        remove_name_from_list(policy, "excluded", &registration.name);
    }
    if registration.provider == "omp"
        && let Some(root) = root.as_object_mut()
    {
        remove_name_from_list(root, "enabledServers", &registration.name);
        remove_name_from_list(root, "disabledServers", &registration.name);
    }

    let mut output = serde_json::to_string_pretty(&root)
        .context("failed to serialize MCP JSON configuration after deletion")?;
    output.push('\n');
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

fn remove_name_from_list(
    root: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    name: &str,
) {
    if let Some(entries) = root.get_mut(key).and_then(serde_json::Value::as_array_mut) {
        entries.retain(|entry| entry.as_str() != Some(name));
    }
}

fn json_server_mut<'a>(
    root: &'a mut serde_json::Value,
    registration: &McpRegistration,
    path: &Path,
    workspace: Option<&Path>,
) -> Result<&'a mut serde_json::Map<String, serde_json::Value>> {
    json_servers_mut(root, registration, path, workspace)?
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

fn json_servers_mut<'a>(
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
    Ok(servers)
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

fn delete_codex_toml(registration: &McpRegistration) -> Result<()> {
    let source_path = editable_source_path(&registration.source)?;
    let content = storage::read_optional_config(&source_path)?.with_context(|| {
        format!(
            "MCP config disappeared before deletion: {}",
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
    if servers.remove(&registration.name).is_none() {
        bail!(
            "MCP registration `{}` is no longer present in {}",
            registration.name,
            source_path.display()
        );
    }
    let output = document.to_string();
    output.parse::<toml::Value>().with_context(|| {
        format!(
            "MCP configuration is invalid TOML after deletion: {}",
            source_path.display()
        )
    })?;
    storage::check_config_size(&source_path, output.len())?;
    crate::fs::atomic_write(&source_path, output.as_bytes())
}

fn toml_registration_offset(content: &str, registration: &McpRegistration) -> Option<usize> {
    content.parse::<DocumentMut>().ok()?;
    let mut offset = 0;
    for line in content.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let Some(header) = toml_table_header(trimmed) else {
            offset += line.len();
            continue;
        };
        if let Ok(keys) = Key::parse(header)
            && keys.len() == 2
            && keys[0] == "mcp_servers"
            && keys[1] == registration.name
        {
            return Some(offset + line.len().saturating_sub(trimmed.len()));
        }
        offset += line.len();
    }
    None
}

fn toml_table_header(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    if bytes.first() != Some(&b'[') || bytes.get(1) == Some(&b'[') {
        return None;
    }
    let mut quote = None;
    let mut escaped = false;
    for (index, byte) in bytes.iter().copied().enumerate().skip(1) {
        if let Some(active_quote) = quote {
            if active_quote == b'"' && byte == b'\\' && !escaped {
                escaped = true;
                continue;
            }
            if byte == active_quote && !escaped {
                quote = None;
            }
            escaped = false;
            continue;
        }
        match byte {
            b'"' | b'\'' => quote = Some(byte),
            b']' => return Some(&line[1..index]),
            _ => {}
        }
    }
    None
}

fn json_registration_offset(
    content: &str,
    registration: &McpRegistration,
    workspace: Option<&Path>,
) -> Option<usize> {
    let root: serde_json::Value = match registration.source_format {
        McpSourceFormat::Json => serde_json::from_str(content).ok()?,
        McpSourceFormat::Jsonc => json5::from_str(content).ok()?,
        _ => return None,
    };
    let path = if registration.provider == "claude" && registration.scope == "local" {
        let project = nearest_claude_project(&root, workspace?)?;
        vec![
            "projects".to_owned(),
            project,
            "mcpServers".to_owned(),
            registration.name.clone(),
        ]
    } else if registration.provider == "opencode" {
        if root
            .get("mcp")
            .and_then(|mcp| mcp.get("servers"))
            .is_some_and(serde_json::Value::is_object)
        {
            vec![
                "mcp".to_owned(),
                "servers".to_owned(),
                registration.name.clone(),
            ]
        } else {
            vec!["mcp".to_owned(), registration.name.clone()]
        }
    } else if root.get("mcpServers").is_some() {
        vec!["mcpServers".to_owned(), registration.name.clone()]
    } else if root.get("mcp_servers").is_some() {
        vec!["mcp_servers".to_owned(), registration.name.clone()]
    } else {
        return None;
    };
    find_json_key_path(content, &path)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum JsonContainer {
    Object,
    Array,
}

struct JsonFrame {
    container: JsonContainer,
    path: Vec<String>,
    pending_key: Option<String>,
}

fn find_json_key_path(content: &str, expected: &[String]) -> Option<usize> {
    let bytes = content.as_bytes();
    let mut stack = Vec::<JsonFrame>::new();
    let mut index = 0;
    while index < bytes.len() {
        index = skip_json_trivia(bytes, index);
        if index >= bytes.len() {
            break;
        }
        match bytes[index] {
            b'{' | b'[' => {
                let container = if bytes[index] == b'{' {
                    JsonContainer::Object
                } else {
                    JsonContainer::Array
                };
                let path = stack.last_mut().map_or_else(Vec::new, |parent| {
                    let mut path = parent.path.clone();
                    if parent.container == JsonContainer::Object
                        && let Some(key) = parent.pending_key.take()
                    {
                        path.push(key);
                    }
                    path
                });
                stack.push(JsonFrame {
                    container,
                    path,
                    pending_key: None,
                });
                index += 1;
            }
            b'}' | b']' => {
                stack.pop();
                index += 1;
            }
            b',' => {
                if let Some(frame) = stack.last_mut() {
                    frame.pending_key = None;
                }
                index += 1;
            }
            b'"' | b'\'' => {
                let token_start = index;
                let token_end = json_quoted_token_end(bytes, index)?;
                let value = if bytes[index] == b'"' {
                    serde_json::from_str::<String>(&content[index..token_end]).ok()?
                } else {
                    json5::from_str::<String>(&content[index..token_end]).ok()?
                };
                index = token_end;
                if record_json_key(bytes, &mut stack, &value, index, expected) {
                    return Some(token_start);
                }
            }
            byte if is_json_identifier_start(byte) => {
                let token_start = index;
                index += 1;
                while bytes
                    .get(index)
                    .copied()
                    .is_some_and(is_json_identifier_continue)
                {
                    index += 1;
                }
                let value = &content[token_start..index];
                if record_json_key(bytes, &mut stack, value, index, expected) {
                    return Some(token_start);
                }
            }
            _ => index += 1,
        }
    }
    None
}

fn record_json_key(
    bytes: &[u8],
    stack: &mut [JsonFrame],
    value: &str,
    token_end: usize,
    expected: &[String],
) -> bool {
    let after = skip_json_trivia(bytes, token_end);
    let Some(frame) = stack.last_mut() else {
        return false;
    };
    if frame.container != JsonContainer::Object || bytes.get(after) != Some(&b':') {
        return false;
    }
    let mut path = frame.path.clone();
    path.push(value.to_owned());
    if path == expected {
        return true;
    }
    frame.pending_key = Some(value.to_owned());
    false
}

fn skip_json_trivia(bytes: &[u8], mut index: usize) -> usize {
    loop {
        while bytes.get(index).is_some_and(u8::is_ascii_whitespace) {
            index += 1;
        }
        if bytes.get(index) == Some(&b'/') && bytes.get(index + 1) == Some(&b'/') {
            index += 2;
            while bytes.get(index).is_some_and(|byte| *byte != b'\n') {
                index += 1;
            }
        } else if bytes.get(index) == Some(&b'/') && bytes.get(index + 1) == Some(&b'*') {
            index += 2;
            while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/') {
                index += 1;
            }
            index = (index + 2).min(bytes.len());
        } else {
            return index;
        }
    }
}

fn json_quoted_token_end(bytes: &[u8], start: usize) -> Option<usize> {
    let quote = *bytes.get(start)?;
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index = index.saturating_add(2),
            byte if byte == quote => return Some(index + 1),
            _ => index += 1,
        }
    }
    None
}

const fn is_json_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || matches!(byte, b'_' | b'$')
}

const fn is_json_identifier_continue(byte: u8) -> bool {
    is_json_identifier_start(byte) || byte.is_ascii_digit() || byte == b'-'
}

fn yaml_registration_offset(content: &str, expected: &str) -> Option<usize> {
    serde_yaml_ng::from_str::<serde_yaml_ng::Value>(content).ok()?;
    let mut offset = 0;
    let mut extensions_indent = None;
    let mut entry_indent = None;
    for line in content.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            offset += line.len();
            continue;
        }
        let indent = line.len().saturating_sub(trimmed.len());
        let key = yaml_mapping_key(trimmed);
        if let Some(root_indent) = extensions_indent {
            if indent <= root_indent {
                break;
            }
            let direct_indent = *entry_indent.get_or_insert(indent);
            if indent == direct_indent && key.as_deref() == Some(expected) {
                return Some(offset + line.len().saturating_sub(trimmed.len()));
            }
        } else if indent == 0 && key.as_deref() == Some("extensions") {
            extensions_indent = Some(indent);
            if trimmed
                .split_once(':')
                .is_some_and(|(_, value)| !value.trim().is_empty())
            {
                return Some(offset);
            }
        }
        offset += line.len();
    }
    None
}

fn yaml_mapping_key(line: &str) -> Option<String> {
    serde_yaml_ng::from_str::<serde_yaml_ng::Value>(line)
        .ok()?
        .as_mapping()?
        .keys()
        .next()?
        .as_str()
        .map(str::to_owned)
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
