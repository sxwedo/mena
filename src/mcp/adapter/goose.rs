use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;

use super::common::{
    RawMcp, RawValue, bool_field, normalize, seconds_to_millis, string_array, string_field,
    unknown_fields,
};
use super::{DiscoveredMcp, storage};
use crate::mcp::{McpSourceFormat, McpTransport};

const KNOWN_FIELDS: &[&str] = &[
    "name",
    "display_name",
    "description",
    "enabled",
    "bundled",
    "type",
    "cmd",
    "command",
    "args",
    "uri",
    "url",
    "env",
    "envs",
    "env_keys",
    "headers",
    "available_tools",
    "timeout",
];

pub(super) fn parse(path: &Path, out: &mut Vec<DiscoveredMcp>) -> Result<()> {
    let Some(content) = storage::read_optional_config(path)? else {
        return Ok(());
    };
    let yaml: serde_yaml_ng::Value = serde_yaml_ng::from_str(&content)
        .with_context(|| format!("failed to parse Goose MCP config {}", path.display()))?;
    let root = serde_json::to_value(yaml)
        .with_context(|| format!("failed to normalize Goose MCP config {}", path.display()))?;
    let Some(extensions) = root.get("extensions").and_then(Value::as_object) else {
        return Ok(());
    };
    storage::check_server_count(path, extensions.len())?;
    for (name, value) in extensions {
        if value.is_object() {
            out.push(normalize_goose(name, path, value));
        }
    }
    Ok(())
}

fn normalize_goose(name: &str, source: &Path, value: &Value) -> DiscoveredMcp {
    let mut raw = RawMcp::new(name, "goose", "user", source, McpSourceFormat::Yaml);
    raw.display_name = string_field(value, "name").or_else(|| string_field(value, "display_name"));
    raw.description = string_field(value, "description");
    raw.enabled = bool_field(value, "enabled", true);
    raw.transport = string_field(value, "type").map(|kind| match kind.as_str() {
        "stdio" => McpTransport::Stdio,
        "streamable_http" | "streamable-http" | "http" => McpTransport::StreamableHttp,
        "sse" => McpTransport::Sse,
        "builtin" => McpTransport::Builtin,
        "platform" => McpTransport::Platform,
        "frontend" => McpTransport::Frontend,
        "inline_python" | "inline-python" => McpTransport::InlinePython,
        _ => McpTransport::Unknown,
    });
    raw.command = string_field(value, "cmd").or_else(|| string_field(value, "command"));
    raw.args = string_array(value.get("args"));
    raw.url = string_field(value, "uri").or_else(|| string_field(value, "url"));
    raw.environment = literal_map(value.get("env").or_else(|| value.get("envs")));
    for name in string_array(value.get("env_keys")) {
        raw.environment.insert(name, RawValue::Forwarded);
    }
    raw.headers = literal_map(value.get("headers"));
    raw.tool_policy.include = string_array(value.get("available_tools"));
    raw.tool_policy.include.sort();
    raw.tool_policy.include.dedup();
    raw.timeouts.catalog_ms = seconds_to_millis(value.get("timeout"));
    if let Some(bundled) = value.get("bundled").filter(|value| value.is_boolean()) {
        raw.options.insert("bundled".to_owned(), bundled.clone());
    }
    raw.extra_fields = unknown_fields(value, KNOWN_FIELDS);
    normalize(raw)
}

fn literal_map(value: Option<&Value>) -> BTreeMap<String, RawValue> {
    value
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|object| object.iter())
        .filter_map(|(name, value)| {
            value
                .as_str()
                .map(|value| (name.clone(), RawValue::Literal(value.to_owned())))
        })
        .collect()
}
