use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;

use super::common::{
    RawMcp, RawValue, bool_field, literal_bindings, millis, normalize, redact_url,
    seconds_to_millis, string_array, string_field, string_map, unknown_fields,
};
use super::{DiscoveredMcp, McpConnection, storage};
use crate::mcp::{McpAuthentication, McpSourceFormat};

const KNOWN_FIELDS: &[&str] = &[
    "command",
    "args",
    "env",
    "env_vars",
    "cwd",
    "url",
    "auth",
    "bearer_token_env_var",
    "http_headers",
    "env_http_headers",
    "startup_timeout_sec",
    "startup_timeout_ms",
    "tool_timeout_sec",
    "enabled",
    "required",
    "enabled_tools",
    "disabled_tools",
    "default_tools_approval_mode",
    "tools",
    "experimental_environment",
    "oauth_resource",
    "scopes",
];

pub(super) fn parse(path: &Path, scope: &'static str, out: &mut Vec<DiscoveredMcp>) -> Result<()> {
    let Some(content) = storage::read_optional_config(path)? else {
        return Ok(());
    };
    let root: toml::Value = content
        .parse()
        .with_context(|| format!("failed to parse Codex MCP config {}", path.display()))?;
    let Some(servers) = root.get("mcp_servers").and_then(toml::Value::as_table) else {
        return Ok(());
    };
    storage::check_server_count(path, servers.len())?;
    for (name, value) in servers {
        let json = serde_json::to_value(value)
            .with_context(|| format!("failed to normalize Codex MCP server `{name}`"))?;
        out.push(normalize_codex(name, scope, path, &json));
    }
    Ok(())
}

fn normalize_codex(name: &str, scope: &'static str, source: &Path, value: &Value) -> DiscoveredMcp {
    let mut raw = RawMcp::new(name, "codex", scope, source, McpSourceFormat::Toml);
    raw.command = string_field(value, "command");
    raw.args = string_array(value.get("args"));
    raw.url = string_field(value, "url");
    raw.cwd = string_field(value, "cwd").map(Into::into);
    raw.enabled = bool_field(value, "enabled", true);
    raw.environment = literal_bindings(value.get("env"));
    let uses_remote_environment = add_forwarded_environments(value.get("env_vars"), &mut raw);
    raw.headers = literal_bindings(value.get("http_headers"));
    for (header, environment) in string_map(value.get("env_http_headers")) {
        raw.headers
            .insert(header, RawValue::Environment(environment));
    }
    if let Some(environment) = string_field(value, "bearer_token_env_var") {
        raw.authentication.push(McpAuthentication {
            kind: "bearer_env".to_owned(),
            reference: Some(environment),
        });
    }
    if raw.url.is_some() {
        let kind = string_field(value, "auth").unwrap_or_else(|| "oauth".to_owned());
        raw.authentication.push(McpAuthentication {
            kind,
            reference: Some("provider credential store".to_owned()),
        });
    }
    raw.timeouts.startup_ms = millis(value.get("startup_timeout_ms"))
        .or_else(|| seconds_to_millis(value.get("startup_timeout_sec")));
    raw.timeouts.tool_ms = seconds_to_millis(value.get("tool_timeout_sec"));
    raw.tool_policy.include = string_array(value.get("enabled_tools"));
    raw.tool_policy.exclude = string_array(value.get("disabled_tools"));
    raw.tool_policy.include.sort();
    raw.tool_policy.include.dedup();
    raw.tool_policy.exclude.sort();
    raw.tool_policy.exclude.dedup();
    raw.tool_policy.default_approval = string_field(value, "default_tools_approval_mode");
    raw.tool_policy.approval_overrides = value
        .get("tools")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|tools| tools.iter())
        .filter_map(|(tool, config)| {
            string_field(config, "approval_mode").map(|mode| (tool.clone(), mode))
        })
        .collect();
    if let Some(required) = value.get("required").filter(|value| value.is_boolean()) {
        raw.options.insert("required".to_owned(), required.clone());
    }
    let execution_environment = string_field(value, "experimental_environment");
    if let Some(environment) = &execution_environment {
        raw.options.insert(
            "execution_environment".to_owned(),
            Value::String(environment.clone()),
        );
    }
    let mut scopes = string_array(value.get("scopes"));
    scopes.sort();
    scopes.dedup();
    if !scopes.is_empty() {
        raw.options.insert(
            "oauth_scopes".to_owned(),
            Value::Array(scopes.into_iter().map(Value::String).collect()),
        );
    }
    if let Some(resource) = string_field(value, "oauth_resource") {
        raw.options.insert(
            "oauth_resource".to_owned(),
            Value::String(redact_url(&resource)),
        );
    }
    raw.extra_fields = unknown_fields(value, KNOWN_FIELDS);
    let mut entry = normalize(raw);
    if execution_environment.as_deref() == Some("remote") || uses_remote_environment {
        let reason =
            "Codex remote executor MCP placement cannot be probed safely by mena".to_owned();
        entry.connection = McpConnection::Unsupported {
            reason: reason.clone(),
        };
        entry.registration.warnings.push(reason);
        entry.registration.warnings.sort();
        entry.registration.warnings.dedup();
    }
    entry
}

fn add_forwarded_environments(value: Option<&Value>, raw: &mut RawMcp) -> bool {
    let mut uses_remote_environment = false;
    for entry in value.and_then(Value::as_array).into_iter().flatten() {
        match entry {
            Value::String(name) => {
                raw.environment.insert(name.clone(), RawValue::Forwarded);
            }
            Value::Object(binding) => {
                let Some(name) = binding.get("name").and_then(Value::as_str) else {
                    raw.warnings
                        .push("Codex env_vars entry is missing a string name".to_owned());
                    continue;
                };
                match binding.get("source").and_then(Value::as_str) {
                    Some("remote") => {
                        uses_remote_environment = true;
                        raw.environment
                            .insert(name.to_owned(), RawValue::RemoteEnvironment);
                    }
                    None | Some("local") => {
                        raw.environment.insert(name.to_owned(), RawValue::Forwarded);
                    }
                    Some(_) => {
                        uses_remote_environment = true;
                        raw.environment
                            .insert(name.to_owned(), RawValue::RemoteEnvironment);
                        raw.warnings.push(
                            "Codex env_vars entry has an unsupported source and cannot be probed"
                                .to_owned(),
                        );
                    }
                }
            }
            _ => raw
                .warnings
                .push("Codex env_vars entry is not a string or object".to_owned()),
        }
    }
    uses_remote_environment
}
