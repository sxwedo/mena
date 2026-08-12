use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

use super::common::{
    RawMcp, RawValue, bool_field, millis, normalize, string_array, string_field, unknown_fields,
};
use super::{DiscoveredMcp, storage};
use crate::mcp::{McpAuthentication, McpSourceFormat, McpTransport};

const STANDARD_FIELDS: &[&str] = &[
    "type",
    "command",
    "args",
    "env",
    "env_vars",
    "environment",
    "cwd",
    "url",
    "httpUrl",
    "headers",
    "headersHelper",
    "enabled",
    "disabled",
    "description",
    "displayName",
    "timeout",
    "trust",
    "alwaysAllow",
    "includeTools",
    "excludeTools",
    "allowedTools",
    "excludedTools",
    "enabledTools",
    "disabledTools",
    "bearerTokenEnvVar",
    "oauth",
    "auth",
    "codemode",
];

pub(super) fn parse_home(
    home: &Path,
    workspace: Option<&Path>,
    out: &mut Vec<DiscoveredMcp>,
) -> Result<()> {
    parse_claude_state(&home.join(".claude.json"), workspace, out)?;
    parse_standard_file(
        &home.join(".cursor/mcp.json"),
        "cursor",
        "user",
        McpSourceFormat::Json,
        Flavor::Cursor,
        out,
    )?;
    parse_standard_file(
        &home.join(".gemini/settings.json"),
        "gemini",
        "user",
        McpSourceFormat::Json,
        Flavor::Gemini,
        out,
    )?;
    for (path, format) in [
        (
            home.join(".config/opencode/opencode.json"),
            McpSourceFormat::Json,
        ),
        (
            home.join(".config/opencode/opencode.jsonc"),
            McpSourceFormat::Jsonc,
        ),
    ] {
        parse_opencode(&path, "user", format, out)?;
    }
    parse_omp(&home.join(".omp/agent/mcp.json"), "user", out)?;
    for profile in storage::child_directories(&home.join(".omp/profiles"))? {
        let start = out.len();
        parse_omp(&profile.join("agent/mcp.json"), "profile", out)?;
        let profile_name = profile
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        for entry in &mut out[start..] {
            entry
                .registration
                .options
                .insert("profile".to_owned(), Value::String(profile_name.clone()));
        }
    }
    if pi_adapter_enabled(&home.join(".pi/agent/settings.json"))? {
        parse_standard_file(
            &home.join(".pi/agent/mcp.json"),
            "pi",
            "user",
            McpSourceFormat::Json,
            Flavor::Pi,
            out,
        )?;
        parse_standard_file(
            &home.join(".config/mcp/mcp.json"),
            "pi",
            "shared",
            McpSourceFormat::Json,
            Flavor::Pi,
            out,
        )?;
    }
    Ok(())
}

pub(super) fn parse_project(
    home: Option<&Path>,
    workspace: &Path,
    out: &mut Vec<DiscoveredMcp>,
) -> Result<()> {
    if let Some(path) = storage::find_nearest(workspace, ".mcp.json", home) {
        parse_standard_file(
            &path,
            "claude",
            "project",
            McpSourceFormat::Json,
            Flavor::Claude,
            out,
        )?;
    }
    if let Some(path) = storage::find_nearest(workspace, ".cursor/mcp.json", home) {
        parse_standard_file(
            &path,
            "cursor",
            "project",
            McpSourceFormat::Json,
            Flavor::Cursor,
            out,
        )?;
    }
    if let Some(path) = storage::find_nearest(workspace, ".gemini/settings.json", home) {
        parse_standard_file(
            &path,
            "gemini",
            "project",
            McpSourceFormat::Json,
            Flavor::Gemini,
            out,
        )?;
    }
    for (relative, format) in [
        ("opencode.json", McpSourceFormat::Json),
        ("opencode.jsonc", McpSourceFormat::Jsonc),
    ] {
        if let Some(path) = storage::find_nearest(workspace, relative, home) {
            parse_opencode(&path, "project", format, out)?;
        }
    }
    if let Some(path) = storage::find_nearest(workspace, ".omp/mcp.json", home) {
        parse_omp(&path, "project", out)?;
    }
    if let Some(home) = home
        && pi_adapter_enabled(&home.join(".pi/agent/settings.json"))?
    {
        for relative in [".pi/mcp.json", ".mcp.json"] {
            if let Some(path) = storage::find_nearest(workspace, relative, Some(home)) {
                parse_standard_file(
                    &path,
                    "pi",
                    "project",
                    McpSourceFormat::Json,
                    Flavor::Pi,
                    out,
                )?;
            }
        }
    }
    Ok(())
}

pub(super) fn parse_claude_managed(path: &Path, out: &mut Vec<DiscoveredMcp>) -> Result<()> {
    parse_standard_file(
        path,
        "claude",
        "managed",
        McpSourceFormat::Json,
        Flavor::Claude,
        out,
    )
}

#[derive(Debug, Clone, Copy)]
pub(super) enum Flavor {
    Claude,
    Cursor,
    Gemini,
    OpenCode,
    Omp,
    Pi,
    Plugin,
}

pub(super) struct PluginContext<'a> {
    pub(super) provider: &'static str,
    pub(super) plugin_id: &'a str,
    pub(super) version: Option<&'a str>,
    pub(super) root: &'a Path,
    pub(super) workspace: Option<&'a Path>,
}

pub(super) fn parse_plugin_file(
    path: &Path,
    context: &PluginContext<'_>,
    out: &mut Vec<DiscoveredMcp>,
) -> Result<()> {
    let Some(root) = read_json(path, McpSourceFormat::Json)? else {
        return Ok(());
    };
    let servers = if root.get("mcpServers").is_some() {
        root.get("mcpServers")
    } else if root.get("mcp_servers").is_some() {
        root.get("mcp_servers")
    } else {
        Some(&root)
    };
    parse_plugin_servers(servers, path, context, out)
}

pub(super) fn parse_plugin_servers(
    servers: Option<&Value>,
    source: &Path,
    context: &PluginContext<'_>,
    out: &mut Vec<DiscoveredMcp>,
) -> Result<()> {
    let Some(servers) = servers.and_then(Value::as_object) else {
        return Ok(());
    };
    storage::check_server_count(source, servers.len())?;
    for (name, value) in servers {
        if !value.is_object() {
            continue;
        }
        let mut entry = normalize_json(
            name,
            context.provider,
            "plugin",
            source,
            McpSourceFormat::Json,
            Flavor::Plugin,
            value,
        );
        configure_plugin_entry(&mut entry, context);
        out.push(entry);
    }
    Ok(())
}

fn parse_claude_state(
    path: &Path,
    workspace: Option<&Path>,
    out: &mut Vec<DiscoveredMcp>,
) -> Result<()> {
    let Some(root) = read_json(path, McpSourceFormat::Json)? else {
        return Ok(());
    };
    parse_server_object(
        root.get("mcpServers"),
        path,
        "claude",
        "user",
        McpSourceFormat::Json,
        Flavor::Claude,
        out,
    )?;
    let Some(workspace) = workspace else {
        return Ok(());
    };
    let project = root
        .get("projects")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|projects| projects.iter())
        .filter(|(project, _)| path_contains(workspace, Path::new(project)))
        .max_by_key(|(project, _)| Path::new(project).components().count())
        .map(|(_, value)| value);
    parse_server_object(
        project.and_then(|value| value.get("mcpServers")),
        path,
        "claude",
        "local",
        McpSourceFormat::Json,
        Flavor::Claude,
        out,
    )
}

fn parse_standard_file(
    path: &Path,
    provider: &'static str,
    scope: &'static str,
    format: McpSourceFormat,
    flavor: Flavor,
    out: &mut Vec<DiscoveredMcp>,
) -> Result<()> {
    let Some(root) = read_json(path, format)? else {
        return Ok(());
    };
    let start = out.len();
    parse_server_object(
        root.get("mcpServers"),
        path,
        provider,
        scope,
        format,
        flavor,
        out,
    )?;
    if matches!(flavor, Flavor::Gemini) {
        apply_gemini_policy(&root, &mut out[start..]);
    }
    Ok(())
}

fn parse_opencode(
    path: &Path,
    scope: &'static str,
    format: McpSourceFormat,
    out: &mut Vec<DiscoveredMcp>,
) -> Result<()> {
    let Some(root) = read_json(path, format)? else {
        return Ok(());
    };
    let servers = root.get("mcp").and_then(|mcp| {
        mcp.get("servers")
            .filter(|servers| servers.is_object())
            .or(Some(mcp))
    });
    parse_server_object(
        servers,
        path,
        "opencode",
        scope,
        format,
        Flavor::OpenCode,
        out,
    )
}

fn parse_omp(path: &Path, scope: &'static str, out: &mut Vec<DiscoveredMcp>) -> Result<()> {
    let Some(root) = read_json(path, McpSourceFormat::Json)? else {
        return Ok(());
    };
    let start = out.len();
    parse_server_object(
        root.get("mcpServers"),
        path,
        "omp",
        scope,
        McpSourceFormat::Json,
        Flavor::Omp,
        out,
    )?;
    let disabled = string_array(root.get("disabledServers"));
    let enabled = string_array(root.get("enabledServers"));
    for entry in &mut out[start..] {
        if disabled.contains(&entry.registration.name) {
            entry.registration.enabled = false;
        }
        if enabled.contains(&entry.registration.name) {
            entry.registration.enabled = true;
        }
    }
    Ok(())
}

fn pi_adapter_enabled(path: &Path) -> Result<bool> {
    let Some(root) = read_json(path, McpSourceFormat::Json)? else {
        return Ok(false);
    };
    Ok(root
        .get("packages")
        .and_then(Value::as_array)
        .is_some_and(|packages| packages.iter().any(mentions_pi_mcp_adapter)))
}

fn mentions_pi_mcp_adapter(value: &Value) -> bool {
    match value {
        Value::String(value) => value.contains("pi-mcp-adapter"),
        Value::Array(values) => values.iter().any(mentions_pi_mcp_adapter),
        Value::Object(values) => values.values().any(mentions_pi_mcp_adapter),
        _ => false,
    }
}

#[allow(clippy::too_many_arguments)]
fn parse_server_object(
    servers: Option<&Value>,
    path: &Path,
    provider: &'static str,
    scope: &'static str,
    format: McpSourceFormat,
    flavor: Flavor,
    out: &mut Vec<DiscoveredMcp>,
) -> Result<()> {
    let Some(servers) = servers.and_then(Value::as_object) else {
        return Ok(());
    };
    storage::check_server_count(path, servers.len())?;
    for (name, value) in servers {
        if value.is_object() {
            out.push(normalize_json(
                name, provider, scope, path, format, flavor, value,
            ));
        }
    }
    Ok(())
}

fn normalize_json(
    name: &str,
    provider: &'static str,
    scope: &'static str,
    source: &Path,
    format: McpSourceFormat,
    flavor: Flavor,
    value: &Value,
) -> DiscoveredMcp {
    let mut raw = RawMcp::new(name, provider, scope, source, format);
    match value.get("command") {
        Some(Value::String(command)) => {
            raw.command = Some(command.clone());
            raw.args = string_array(value.get("args"));
        }
        Some(Value::Array(command)) if matches!(flavor, Flavor::OpenCode) => {
            let mut parts = command.iter().filter_map(Value::as_str).map(str::to_owned);
            raw.command = parts.next();
            raw.args = parts.collect();
        }
        _ => {}
    }
    raw.url = string_field(value, "httpUrl").or_else(|| string_field(value, "url"));
    raw.transport = transport(value, flavor);
    raw.cwd = string_field(value, "cwd").map(PathBuf::from);
    raw.enabled = bool_field(value, "enabled", true) && !bool_field(value, "disabled", false);
    raw.description = string_field(value, "description");
    raw.display_name = string_field(value, "displayName");
    raw.environment = parse_bindings(
        value
            .get("env")
            .or_else(|| value.get("environment"))
            .and_then(Value::as_object),
    );
    for name in string_array(value.get("env_vars")) {
        raw.environment.insert(name, RawValue::Forwarded);
    }
    raw.headers = parse_bindings(value.get("headers").and_then(Value::as_object));
    if value.get("headersHelper").is_some() {
        raw.headers
            .insert("<headersHelper>".to_owned(), RawValue::DynamicCommand);
    }
    if let Some(environment) = string_field(value, "bearerTokenEnvVar") {
        raw.authentication.push(McpAuthentication {
            kind: "bearer_env".to_owned(),
            reference: Some(environment),
        });
    }
    if value
        .get("oauth")
        .is_some_and(|oauth| oauth != &Value::Bool(false))
    {
        raw.authentication.push(McpAuthentication {
            kind: "oauth".to_owned(),
            reference: Some("provider credential store".to_owned()),
        });
    }
    if let Some(kind) = string_field(value, "auth") {
        raw.authentication.push(McpAuthentication {
            kind,
            reference: Some("provider credential store".to_owned()),
        });
    }
    normalize_timeouts(value.get("timeout"), &mut raw);
    raw.tool_policy.include = first_array(value, &["includeTools", "allowedTools", "enabledTools"]);
    raw.tool_policy.exclude =
        first_array(value, &["excludeTools", "excludedTools", "disabledTools"]);
    raw.tool_policy.include.sort();
    raw.tool_policy.include.dedup();
    raw.tool_policy.exclude.sort();
    raw.tool_policy.exclude.dedup();
    for key in ["trust", "alwaysAllow", "codemode"] {
        if let Some(option) = value.get(key).filter(|value| {
            value.is_boolean() || value.is_number() || value.is_array() && key == "alwaysAllow"
        }) {
            raw.options.insert(key.to_owned(), option.clone());
        }
    }
    raw.extra_fields = unknown_fields(value, STANDARD_FIELDS);
    normalize(raw)
}

fn apply_gemini_policy(root: &Value, registrations: &mut [DiscoveredMcp]) {
    let policy = root.get("mcp");
    let allowed = string_array(policy.and_then(|policy| policy.get("allowed")));
    let excluded = string_array(policy.and_then(|policy| policy.get("excluded")));
    for entry in registrations {
        let name = &entry.registration.name;
        let allowed_by_policy = allowed.is_empty() || allowed.contains(name);
        let excluded_by_policy = excluded.contains(name);
        if !allowed_by_policy || excluded_by_policy {
            entry.registration.enabled = false;
            entry
                .registration
                .options
                .insert("disabled_by_global_policy".to_owned(), Value::Bool(true));
        }
    }
}

fn normalize_timeouts(value: Option<&Value>, raw: &mut RawMcp) {
    let Some(value) = value else {
        return;
    };
    if let Some(timeout) = millis(Some(value)) {
        raw.timeouts.catalog_ms = Some(timeout);
        return;
    }
    let Some(timeouts) = value.as_object() else {
        return;
    };
    raw.timeouts.startup_ms = first_millis(timeouts, &["startup", "connect", "initialize"]);
    raw.timeouts.catalog_ms = first_millis(timeouts, &["catalog", "request", "list"]);
    raw.timeouts.tool_ms = first_millis(timeouts, &["tool", "call"]);
}

fn first_millis(values: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| values.get(*key).and_then(Value::as_u64))
}

fn configure_plugin_entry(entry: &mut DiscoveredMcp, context: &PluginContext<'_>) {
    entry.registration.options.insert(
        "plugin_id".to_owned(),
        Value::String(context.plugin_id.to_owned()),
    );
    if let Some(version) = context.version {
        entry.registration.options.insert(
            "plugin_version".to_owned(),
            Value::String(version.to_owned()),
        );
    }
    let mut variables = BTreeMap::new();
    match context.provider {
        "claude" => {
            variables.insert(
                "CLAUDE_PLUGIN_ROOT".to_owned(),
                context.root.display().to_string(),
            );
            if let Some(workspace) = context.workspace {
                variables.insert(
                    "CLAUDE_PROJECT_DIR".to_owned(),
                    workspace.display().to_string(),
                );
            }
        }
        "codex" => {
            variables.insert(
                "CODEX_PLUGIN_ROOT".to_owned(),
                context.root.display().to_string(),
            );
            if let Some(workspace) = context.workspace {
                variables.insert(
                    "CODEX_PROJECT_DIR".to_owned(),
                    workspace.display().to_string(),
                );
            }
        }
        _ => {}
    }
    match &mut entry.connection {
        super::McpConnection::Stdio {
            cwd,
            variables: connection_variables,
            ..
        } => {
            let effective_cwd = cwd.take().map_or_else(
                || context.root.to_path_buf(),
                |cwd| {
                    if cwd.is_absolute() {
                        cwd
                    } else {
                        context.root.join(cwd)
                    }
                },
            );
            let effective_cwd = effective_cwd.canonicalize().unwrap_or(effective_cwd);
            *cwd = Some(effective_cwd.clone());
            entry.registration.cwd = Some(effective_cwd);
            connection_variables.extend(variables);
        }
        super::McpConnection::Http {
            variables: connection_variables,
            ..
        } => connection_variables.extend(variables),
        super::McpConnection::Unsupported { .. } => {}
    }
}

fn transport(value: &Value, flavor: Flavor) -> Option<McpTransport> {
    let configured = string_field(value, "type").map(|kind| kind.to_ascii_lowercase());
    match configured.as_deref() {
        Some("stdio" | "local") => Some(McpTransport::Stdio),
        Some("http" | "remote" | "streamable-http" | "streamable_http") => {
            Some(McpTransport::StreamableHttp)
        }
        Some("sse") => Some(McpTransport::Sse),
        Some(_) => Some(McpTransport::Unknown),
        None if value.get("httpUrl").is_some() => Some(McpTransport::StreamableHttp),
        None if matches!(flavor, Flavor::Gemini) && value.get("url").is_some() => {
            Some(McpTransport::Sse)
        }
        None => None,
    }
}

fn parse_bindings(object: Option<&serde_json::Map<String, Value>>) -> BTreeMap<String, RawValue> {
    object
        .into_iter()
        .flat_map(|object| object.iter())
        .filter_map(|(name, value)| {
            value
                .as_str()
                .map(|value| (name.clone(), classify_value(value)))
        })
        .collect()
}

fn classify_value(value: &str) -> RawValue {
    if value.starts_with('!') {
        return RawValue::DynamicCommand;
    }
    let environments = environment_references(value);
    match environments.as_slice() {
        [] => RawValue::Literal(value.to_owned()),
        [environment]
            if value == format!("${environment}")
                || value == format!("${{{environment}}}")
                || value == format!("{{env:{environment}}}") =>
        {
            RawValue::Environment(environment.clone())
        }
        _ => RawValue::EnvironmentTemplate {
            template: value.to_owned(),
            environments,
        },
    }
}

fn environment_references(value: &str) -> Vec<String> {
    let bytes = value.as_bytes();
    let mut references = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'{' && value[index..].starts_with("{env:") {
            let start = index + 5;
            if let Some(relative_end) = value[start..].find('}') {
                let end = start + relative_end;
                let name = &value[start..end];
                if valid_environment_name(name) {
                    references.push(name.to_owned());
                }
                index = end + 1;
                continue;
            }
        }
        if bytes[index] != b'$' {
            index += 1;
            continue;
        }
        index += 1;
        if index >= bytes.len() {
            break;
        }
        let (start, end) = if bytes[index] == b'{' {
            let start = index + 1;
            let Some(relative_end) = bytes[start..].iter().position(|byte| *byte == b'}') else {
                break;
            };
            let end = start + relative_end;
            index = end + 1;
            let body = &value[start..end];
            let name = body.split_once(":-").map_or(body, |(name, _)| name);
            if valid_environment_name(name) {
                references.push(name.to_owned());
            }
            continue;
        } else {
            let start = index;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            (start, index)
        };
        if start < end {
            let name = &value[start..end];
            if valid_environment_name(name) {
                references.push(name.to_owned());
            }
        }
    }
    references.sort();
    references.dedup();
    references
}

fn valid_environment_name(name: &str) -> bool {
    !name.is_empty()
        && (name.as_bytes()[0].is_ascii_alphabetic() || name.as_bytes()[0] == b'_')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn first_array(value: &Value, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .find_map(|key| value.get(*key).filter(|value| value.is_array()))
        .map_or_else(Vec::new, |value| string_array(Some(value)))
}

fn read_json(path: &Path, format: McpSourceFormat) -> Result<Option<Value>> {
    let Some(content) = storage::read_optional_config(path)? else {
        return Ok(None);
    };
    let value = if format == McpSourceFormat::Jsonc {
        json5::from_str(&content)
            .with_context(|| format!("failed to parse MCP JSONC config {}", path.display()))?
    } else {
        serde_json::from_str(&content)
            .with_context(|| format!("failed to parse MCP JSON config {}", path.display()))?
    };
    Ok(Some(value))
}

fn path_contains(path: &Path, ancestor: &Path) -> bool {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let ancestor = ancestor
        .canonicalize()
        .unwrap_or_else(|_| ancestor.to_path_buf());
    path.starts_with(ancestor)
}

#[cfg(test)]
mod tests {
    use super::{RawValue, classify_value};

    #[test]
    fn classifies_environment_references_without_treating_them_as_literals() {
        assert!(matches!(
            classify_value("${API_TOKEN}"),
            RawValue::Environment(environment) if environment == "API_TOKEN"
        ));
        assert!(matches!(
            classify_value("Bearer ${API_TOKEN}"),
            RawValue::EnvironmentTemplate { template, environments }
                if template == "Bearer ${API_TOKEN}" && environments == ["API_TOKEN"]
        ));
        assert!(matches!(
            classify_value("{env:API_TOKEN}"),
            RawValue::Environment(environment) if environment == "API_TOKEN"
        ));
        assert!(matches!(
            classify_value("${API_HOST:-https://example.test}"),
            RawValue::EnvironmentTemplate { environments, .. }
                if environments == ["API_HOST"]
        ));
        assert!(matches!(
            classify_value("!security lookup token"),
            RawValue::DynamicCommand
        ));
    }
}
