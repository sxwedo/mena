use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{DiscoveredMcp, McpConnection};
use crate::mcp::{
    McpAuthentication, McpRegistration, McpSourceFormat, McpTimeouts, McpToolPolicy, McpTransport,
    McpValueBinding, McpValueSource,
};

#[derive(Clone)]
pub(super) enum RawValue {
    Literal(String),
    Environment(String),
    EnvironmentTemplate {
        template: String,
        environments: Vec<String>,
    },
    Forwarded,
    RemoteEnvironment,
    DynamicCommand,
}

impl RawValue {
    const fn source(&self) -> McpValueSource {
        match self {
            Self::Literal(_) => McpValueSource::Literal,
            Self::Environment(_) | Self::EnvironmentTemplate { .. } => McpValueSource::Environment,
            Self::Forwarded => McpValueSource::Forwarded,
            Self::RemoteEnvironment => McpValueSource::RemoteEnvironment,
            Self::DynamicCommand => McpValueSource::DynamicCommand,
        }
    }
}

pub(super) struct RawMcp {
    pub(super) name: String,
    pub(super) provider: &'static str,
    pub(super) scope: &'static str,
    pub(super) source: PathBuf,
    pub(super) source_format: McpSourceFormat,
    pub(super) transport: Option<McpTransport>,
    pub(super) enabled: bool,
    pub(super) display_name: Option<String>,
    pub(super) description: Option<String>,
    pub(super) command: Option<String>,
    pub(super) args: Vec<String>,
    pub(super) url: Option<String>,
    pub(super) cwd: Option<PathBuf>,
    pub(super) timeouts: McpTimeouts,
    pub(super) authentication: Vec<McpAuthentication>,
    pub(super) environment: BTreeMap<String, RawValue>,
    pub(super) headers: BTreeMap<String, RawValue>,
    pub(super) tool_policy: McpToolPolicy,
    pub(super) options: BTreeMap<String, Value>,
    pub(super) variables: BTreeMap<String, String>,
    pub(super) extra_fields: Vec<String>,
    pub(super) warnings: Vec<String>,
}

impl RawMcp {
    pub(super) fn new(
        name: &str,
        provider: &'static str,
        scope: &'static str,
        source: &Path,
        source_format: McpSourceFormat,
    ) -> Self {
        Self {
            name: name.to_owned(),
            provider,
            scope,
            source: source.to_path_buf(),
            source_format,
            transport: None,
            enabled: true,
            display_name: None,
            description: None,
            command: None,
            args: Vec::new(),
            url: None,
            cwd: None,
            timeouts: McpTimeouts::default(),
            authentication: Vec::new(),
            environment: BTreeMap::new(),
            headers: BTreeMap::new(),
            tool_policy: McpToolPolicy::default(),
            options: BTreeMap::new(),
            variables: BTreeMap::new(),
            extra_fields: Vec::new(),
            warnings: Vec::new(),
        }
    }
}

pub(super) fn normalize(mut raw: RawMcp) -> DiscoveredMcp {
    let transport = normalize_transport_and_warnings(&mut raw);
    let environment = sorted_public_bindings(&raw.environment);
    let headers = sorted_public_bindings(&raw.headers);
    let connection = connection(&raw, transport);
    let registration = into_registration(raw, transport, environment, headers);
    DiscoveredMcp {
        registration,
        connection,
    }
}

fn normalize_transport_and_warnings(raw: &mut RawMcp) -> McpTransport {
    let transport = raw.transport.unwrap_or(match (&raw.command, &raw.url) {
        (Some(_), None) => McpTransport::Stdio,
        (None, Some(_)) => McpTransport::StreamableHttp,
        _ => McpTransport::Unknown,
    });
    match (&raw.command, &raw.url) {
        (Some(_), Some(_)) => raw
            .warnings
            .push("both command and url are configured; transport is ambiguous".to_owned()),
        (None, None)
            if !matches!(
                transport,
                McpTransport::Builtin | McpTransport::Platform | McpTransport::Frontend
            ) =>
        {
            raw.warnings
                .push("neither command nor url is configured".to_owned());
        }
        _ => {}
    }
    if raw
        .environment
        .values()
        .chain(raw.headers.values())
        .any(|value| matches!(value, RawValue::DynamicCommand))
    {
        raw.warnings
            .push("dynamic value commands are recorded but never executed by mena".to_owned());
    }
    raw.extra_fields.sort();
    raw.extra_fields.dedup();
    raw.warnings.sort();
    raw.warnings.dedup();
    transport
}

fn connection(raw: &RawMcp, transport: McpTransport) -> McpConnection {
    match (&raw.command, &raw.url, transport) {
        (Some(command), None, McpTransport::Stdio) => McpConnection::Stdio {
            command: command.clone(),
            args: raw.args.clone(),
            environment: literal_values(&raw.environment),
            environment_sources: environment_sources(&raw.environment),
            environment_templates: environment_templates(&raw.environment),
            variables: raw.variables.clone(),
            unresolved_values: dynamic_value_names(&raw.environment),
            cwd: raw.cwd.clone(),
        },
        (None, Some(url), McpTransport::StreamableHttp) => McpConnection::Http {
            url: url.clone(),
            headers: literal_values(&raw.headers),
            environment_headers: environment_sources(&raw.headers),
            environment_templates: environment_templates(&raw.headers),
            variables: raw.variables.clone(),
            bearer_token_environment: raw
                .authentication
                .iter()
                .find(|auth| auth.kind == "bearer_env")
                .and_then(|auth| auth.reference.clone()),
            unresolved_values: dynamic_value_names(&raw.headers),
        },
        _ => McpConnection::Unsupported {
            reason: format!("{} transport cannot be probed by mena", transport.as_str()),
        },
    }
}

fn into_registration(
    raw: RawMcp,
    transport: McpTransport,
    environment: Vec<McpValueBinding>,
    headers: Vec<McpValueBinding>,
) -> McpRegistration {
    let valid = raw.warnings.iter().all(|warning| {
        !warning.starts_with("both command") && !warning.starts_with("neither command")
    });
    McpRegistration {
        selector: format!("{}:{}:{}", raw.provider, raw.scope, raw.name),
        name: raw.name,
        provider: raw.provider.to_owned(),
        scope: raw.scope.to_owned(),
        source: raw.source,
        source_format: raw.source_format,
        transport,
        enabled: raw.enabled,
        valid,
        display_name: raw.display_name,
        description: raw.description.map(|value| bounded_text(&value)),
        command: raw.command.as_deref().map(redact_inline_secret),
        args: redact_args(&raw.args),
        url: raw.url.as_deref().map(redact_url),
        cwd: raw.cwd,
        timeouts: raw.timeouts,
        authentication: raw.authentication,
        environment,
        headers,
        tool_policy: raw.tool_policy,
        options: raw.options,
        extra_fields: raw.extra_fields,
        warnings: raw.warnings,
    }
}

fn sorted_public_bindings(values: &BTreeMap<String, RawValue>) -> Vec<McpValueBinding> {
    let mut bindings = public_bindings(values);
    bindings.sort_by(|left, right| left.name.cmp(&right.name));
    bindings
}

fn environment_sources(values: &BTreeMap<String, RawValue>) -> BTreeMap<String, String> {
    values
        .iter()
        .filter_map(|(name, value)| match value {
            RawValue::Forwarded => Some((name.clone(), name.clone())),
            RawValue::Environment(environment) => Some((name.clone(), environment.clone())),
            _ => None,
        })
        .collect()
}

fn environment_templates(
    values: &BTreeMap<String, RawValue>,
) -> BTreeMap<String, (String, Vec<String>)> {
    values
        .iter()
        .filter_map(|(name, value)| match value {
            RawValue::EnvironmentTemplate {
                template,
                environments,
            } => Some((name.clone(), (template.clone(), environments.clone()))),
            _ => None,
        })
        .collect()
}

fn dynamic_value_names(values: &BTreeMap<String, RawValue>) -> Vec<String> {
    values
        .iter()
        .filter(|(_, value)| matches!(value, RawValue::DynamicCommand))
        .map(|(name, _)| name.clone())
        .collect()
}

fn public_bindings(values: &BTreeMap<String, RawValue>) -> Vec<McpValueBinding> {
    values
        .iter()
        .map(|(name, value)| McpValueBinding {
            name: name.clone(),
            source: value.source(),
            sensitive: is_sensitive_name(name),
        })
        .collect()
}

fn literal_values(values: &BTreeMap<String, RawValue>) -> BTreeMap<String, String> {
    values
        .iter()
        .filter_map(|(name, value)| match value {
            RawValue::Literal(value) => Some((name.clone(), value.clone())),
            _ => None,
        })
        .collect()
}

pub(super) fn string_field(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

pub(super) fn bool_field(value: &Value, key: &str, default: bool) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(default)
}

pub(super) fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

pub(super) fn string_map(value: Option<&Value>) -> BTreeMap<String, String> {
    value
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|object| object.iter())
        .filter_map(|(key, value)| value.as_str().map(|value| (key.clone(), value.to_owned())))
        .collect()
}

pub(super) fn literal_bindings(value: Option<&Value>) -> BTreeMap<String, RawValue> {
    string_map(value)
        .into_iter()
        .map(|(name, value)| (name, RawValue::Literal(value)))
        .collect()
}

pub(super) fn unknown_fields(value: &Value, known: &[&str]) -> Vec<String> {
    let known: BTreeSet<&str> = known.iter().copied().collect();
    value
        .as_object()
        .into_iter()
        .flat_map(|object| object.keys())
        .filter(|key| !known.contains(key.as_str()))
        .cloned()
        .collect()
}

pub(super) fn millis(value: Option<&Value>) -> Option<u64> {
    value.and_then(Value::as_u64)
}

pub(super) fn seconds_to_millis(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    if let Some(seconds) = value.as_u64() {
        return seconds.checked_mul(1_000);
    }
    let seconds = value.as_f64()?;
    let duration = std::time::Duration::try_from_secs_f64(seconds).ok()?;
    u64::try_from(duration.as_millis()).ok()
}

fn is_sensitive_name(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase();
    [
        "token",
        "secret",
        "password",
        "passwd",
        "api_key",
        "api-key",
        "apikey",
        "authorization",
        "credential",
        "private_key",
        "private-key",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

fn redact_args(args: &[String]) -> Vec<String> {
    let mut redact_next = false;
    args.iter()
        .map(|arg| {
            if redact_next {
                redact_next = false;
                return "<redacted>".to_owned();
            }
            let normalized = arg.to_ascii_lowercase();
            let flag = normalized.trim_start_matches('-');
            let sensitive_flag = normalized.starts_with('-')
                && ([
                    "token",
                    "secret",
                    "password",
                    "passwd",
                    "api-key",
                    "apikey",
                    "authorization",
                    "credential",
                    "private-key",
                ]
                .iter()
                .any(|marker| flag.contains(marker))
                    || matches!(flag, "h" | "header" | "env" | "e"));
            if sensitive_flag {
                if let Some((flag, _)) = arg.split_once('=') {
                    return format!("{flag}=<redacted>");
                }
                redact_next = true;
            }
            redact_inline_secret(arg)
        })
        .collect()
}

fn redact_inline_secret(value: &str) -> String {
    if url::Url::parse(value).is_ok_and(|url| matches!(url.scheme(), "http" | "https")) {
        return redact_url(value);
    }
    redact_inline_assignment(value)
}

fn redact_inline_assignment(value: &str) -> String {
    let normalized = value.to_ascii_lowercase();
    if normalized.starts_with("authorization:") || normalized.starts_with("bearer ") {
        return "<redacted>".to_owned();
    }
    for marker in ["token=", "secret=", "password=", "api_key=", "apikey="] {
        if let Some(index) = normalized.find(marker) {
            let prefix_end = index + marker.len();
            return format!("{}<redacted>", &value[..prefix_end]);
        }
    }
    value.to_owned()
}

pub(super) fn redact_url(value: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(value) else {
        return value.split_once('?').map_or_else(
            || redact_inline_assignment(value),
            |(base, _)| base.to_owned(),
        );
    };
    let query_names: Vec<String> = parsed
        .query_pairs()
        .map(|(name, _)| name.into_owned())
        .collect();
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    parsed.set_fragment(None);
    parsed.set_query(None);
    let mut public = parsed.to_string();
    if !query_names.is_empty() {
        public.push('?');
        public.push_str(
            &query_names
                .into_iter()
                .map(|name| format!("{name}=<redacted>"))
                .collect::<Vec<_>>()
                .join("&"),
        );
    }
    public
}

fn bounded_text(value: &str) -> String {
    const MAX_METADATA_CHARS: usize = 64 * 1_024;
    value.chars().take(MAX_METADATA_CHARS).collect()
}
