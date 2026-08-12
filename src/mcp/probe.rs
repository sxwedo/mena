use std::collections::{BTreeSet, HashMap};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use http::{HeaderName, HeaderValue};
use rmcp::model::{
    ClientCapabilities, ClientInfo, Implementation, MetaObject, PaginatedRequestParams, Prompt,
    Resource, ResourceTemplate, ServerPeerInfo, Tool,
};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{Peer, RoleClient, ServiceExt};

use super::adapter::McpConnection;
use super::{
    McpListCapability, McpProbe, McpProbeStatus, McpPromptArgument, McpRegistration,
    McpResourceCapability, McpRuntimePrompt, McpRuntimeResource, McpRuntimeResourceTemplate,
    McpRuntimeTool, McpServerCapabilities, McpServerIdentity, McpToolAnnotations,
};

const MAX_CATALOG_ITEMS: usize = 10_000;
const MAX_PAGES: usize = 1_000;
const MAX_TEXT_CHARS: usize = 64 * 1_024;
const MAX_SCHEMA_BYTES: usize = 1_024 * 1_024;
const MAX_ERROR_CHARS: usize = 4 * 1_024;
const SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(250);
const SAFE_BASE_ENVIRONMENT: &[&str] = &[
    "HOME",
    "LANG",
    "LC_ALL",
    "LOGNAME",
    "PATH",
    "SHELL",
    "TEMP",
    "TMP",
    "TMPDIR",
    "USER",
    "XDG_CACHE_HOME",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
];

fn client_info() -> ClientInfo {
    ClientInfo::new(
        ClientCapabilities::default(),
        Implementation::new("mena", env!("CARGO_PKG_VERSION"))
            .with_title("Mena MCP metadata probe")
            .with_description("Read-only MCP capability and catalog discovery"),
    )
}

pub(super) fn run(
    connection: &McpConnection,
    registration: &McpRegistration,
    timeout: Duration,
) -> McpProbe {
    let started = Instant::now();
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            return failed(
                started,
                format!("could not start MCP probe runtime: {error}"),
            );
        }
    };
    let result = runtime.block_on(async {
        tokio::time::timeout(timeout, run_async(connection, registration, started)).await
    });
    match result {
        Ok(Ok(probe)) => probe,
        Ok(Err(error)) => failed(
            started,
            sanitize_error(&error.to_string(), connection, registration),
        ),
        Err(_) => {
            // Dropping an in-flight child transport schedules process cleanup.
            // Give that task one scheduler turn before tearing down the runtime.
            runtime.block_on(async { tokio::task::yield_now().await });
            failed(
                started,
                format!("live MCP probe exceeded the {}s timeout", timeout.as_secs()),
            )
        }
    }
}

async fn run_async(
    connection: &McpConnection,
    registration: &McpRegistration,
    started: Instant,
) -> Result<McpProbe> {
    match connection {
        McpConnection::Stdio { .. } => run_stdio(connection, registration, started).await,
        McpConnection::Http { .. } => run_http(connection, registration, started).await,
        McpConnection::Unsupported { reason } => bail!("{reason}"),
    }
}

async fn run_stdio(
    connection: &McpConnection,
    registration: &McpRegistration,
    started: Instant,
) -> Result<McpProbe> {
    let McpConnection::Stdio {
        command,
        args,
        environment,
        environment_sources,
        environment_templates,
        variables,
        unresolved_values,
        cwd,
    } = connection
    else {
        bail!("internal MCP connection type mismatch")
    };
    reject_dynamic_values(unresolved_values)?;
    let command = resolve_config_value(command, variables, "command")?;
    let command = resolve_executable(&command, cwd.as_deref(), variables)?;
    let args = args
        .iter()
        .map(|arg| resolve_config_value(arg, variables, "argument"))
        .collect::<Result<Vec<_>>>()?;
    let mut process = tokio::process::Command::new(&command);
    process
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .env_clear();
    for name in SAFE_BASE_ENVIRONMENT {
        if let Some(value) = std::env::var_os(name) {
            process.env(name, value);
        }
    }
    process.envs(environment);
    for (name, source) in environment_sources {
        let value = std::env::var_os(source).with_context(|| {
            format!("required environment variable `{source}` for `{name}` is not set")
        })?;
        process.env(name, value);
    }
    for (name, (template, environments)) in environment_templates {
        process.env(
            name,
            resolve_template(template, environments, variables, name)?,
        );
    }
    if let Some(cwd) = cwd {
        let cwd = resolve_config_value(&cwd.to_string_lossy(), variables, "cwd")?;
        process.current_dir(cwd);
    }
    let (transport, _) = TokioChildProcess::builder(process)
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to start configured MCP command `{command}`"))?;
    let mut client = client_info()
        .serve(transport)
        .await
        .context("MCP initialization failed")?;
    let result = collect(client.peer(), registration, started).await;
    let shutdown = client.close_with_timeout(SHUTDOWN_TIMEOUT).await;
    finish_with_shutdown(result, shutdown)
}

async fn run_http(
    connection: &McpConnection,
    registration: &McpRegistration,
    started: Instant,
) -> Result<McpProbe> {
    let McpConnection::Http {
        url,
        headers,
        environment_headers,
        environment_templates,
        variables,
        bearer_token_environment,
        unresolved_values,
    } = connection
    else {
        bail!("internal MCP connection type mismatch")
    };
    reject_dynamic_values(unresolved_values)?;
    let mut public_headers = HashMap::new();
    for (name, value) in headers {
        public_headers.insert(parse_header_name(name)?, parse_header_value(name, value)?);
    }
    for (name, environment) in environment_headers {
        let value = std::env::var(environment).with_context(|| {
            format!(
                "required environment variable `{environment}` for header `{name}` is not set or is not Unicode"
            )
        })?;
        public_headers.insert(parse_header_name(name)?, parse_header_value(name, &value)?);
    }
    for (name, (template, environments)) in environment_templates {
        let value = resolve_template(template, environments, variables, name)?;
        public_headers.insert(parse_header_name(name)?, parse_header_value(name, &value)?);
    }
    let url = resolve_config_value(url, variables, "url")?;
    let mut config =
        StreamableHttpClientTransportConfig::with_uri(url).custom_headers(public_headers);
    if let Some(environment) = bearer_token_environment {
        let token = std::env::var(environment).with_context(|| {
            format!(
                "required bearer-token environment variable `{environment}` is not set or is not Unicode"
            )
        })?;
        config = config.auth_header(token);
    }
    let transport = StreamableHttpClientTransport::from_config(config);
    let mut client = client_info()
        .serve(transport)
        .await
        .context("MCP initialization failed")?;
    let result = collect(client.peer(), registration, started).await;
    let shutdown = client.close_with_timeout(SHUTDOWN_TIMEOUT).await;
    finish_with_shutdown(result, shutdown)
}

fn reject_dynamic_values(unresolved_values: &[String]) -> Result<()> {
    if !unresolved_values.is_empty() {
        bail!(
            "dynamic value commands for {} are never executed; replace them with explicit environment references",
            unresolved_values.join(", ")
        );
    }
    Ok(())
}

fn finish_with_shutdown(
    result: Result<McpProbe>,
    shutdown: Result<Option<rmcp::service::QuitReason>, tokio::task::JoinError>,
) -> Result<McpProbe> {
    let mut probe = result?;
    match shutdown {
        Ok(Some(_)) => {}
        Ok(None) => probe
            .warnings
            .push("MCP transport cleanup exceeded 250ms".to_owned()),
        Err(error) => probe
            .warnings
            .push(format!("MCP transport cleanup task failed: {error}")),
    }
    if !probe.warnings.is_empty() {
        probe.status = McpProbeStatus::Partial;
    }
    Ok(probe)
}

async fn collect(
    peer: &Peer<RoleClient>,
    registration: &McpRegistration,
    started: Instant,
) -> Result<McpProbe> {
    let info = peer
        .peer_info()
        .context("MCP server completed initialization without peer metadata")?;
    let mut warnings = Vec::new();
    let mut tools = Vec::new();
    let mut prompts = Vec::new();
    let mut resources = Vec::new();
    let mut resource_templates = Vec::new();

    if info.capabilities.tools.is_some() {
        match list_tools(peer).await {
            Ok(items) => tools = normalize_tools(items, registration)?,
            Err(error) => warnings.push(format!("tools/list failed: {error}")),
        }
    }
    if info.capabilities.prompts.is_some() {
        match list_prompts(peer).await {
            Ok(items) => prompts = items.into_iter().map(normalize_prompt).collect(),
            Err(error) => warnings.push(format!("prompts/list failed: {error}")),
        }
    }
    if info.capabilities.resources.is_some() {
        match list_resources(peer).await {
            Ok(items) => resources = items.iter().map(normalize_resource).collect(),
            Err(error) => warnings.push(format!("resources/list failed: {error}")),
        }
        match list_resource_templates(peer).await {
            Ok(items) => {
                resource_templates = items.iter().map(normalize_resource_template).collect();
            }
            Err(error) => warnings.push(format!("resources/templates/list failed: {error}")),
        }
    }
    tools.sort_by(|left, right| left.name.cmp(&right.name));
    prompts.sort_by(|left, right| left.name.cmp(&right.name));
    resources.sort_by(|left, right| left.uri.cmp(&right.uri));
    resource_templates.sort_by(|left, right| left.uri_template.cmp(&right.uri_template));

    Ok(McpProbe {
        status: if warnings.is_empty() {
            McpProbeStatus::Success
        } else {
            McpProbeStatus::Partial
        },
        duration_ms: elapsed_millis(started),
        protocol_version: Some(info.protocol_version.as_str().to_owned()),
        server: info.server_info.as_ref().map(normalize_server_identity),
        capabilities: Some(normalize_capabilities(&info)),
        instructions: info.instructions.as_deref().map(bounded_text),
        tools,
        prompts,
        resources,
        resource_templates,
        warnings,
        error: None,
    })
}

async fn list_tools(peer: &Peer<RoleClient>) -> Result<Vec<Tool>> {
    let mut items = Vec::new();
    let mut cursor = None;
    let mut seen = BTreeSet::new();
    for _ in 0..MAX_PAGES {
        let result = peer
            .list_tools(Some(PaginatedRequestParams::default().with_cursor(cursor)))
            .await?;
        checked_extend(&mut items, result.tools, "tools")?;
        let Some(next) = result.next_cursor else {
            return Ok(items);
        };
        if !seen.insert(next.clone()) {
            bail!("tools/list repeated cursor `{next}`");
        }
        cursor = Some(next);
    }
    bail!("tools/list exceeded the {MAX_PAGES} page limit")
}

async fn list_prompts(peer: &Peer<RoleClient>) -> Result<Vec<Prompt>> {
    let mut items = Vec::new();
    let mut cursor = None;
    let mut seen = BTreeSet::new();
    for _ in 0..MAX_PAGES {
        let result = peer
            .list_prompts(Some(PaginatedRequestParams::default().with_cursor(cursor)))
            .await?;
        checked_extend(&mut items, result.prompts, "prompts")?;
        let Some(next) = result.next_cursor else {
            return Ok(items);
        };
        if !seen.insert(next.clone()) {
            bail!("prompts/list repeated cursor `{next}`");
        }
        cursor = Some(next);
    }
    bail!("prompts/list exceeded the {MAX_PAGES} page limit")
}

async fn list_resources(peer: &Peer<RoleClient>) -> Result<Vec<Resource>> {
    let mut items = Vec::new();
    let mut cursor = None;
    let mut seen = BTreeSet::new();
    for _ in 0..MAX_PAGES {
        let result = peer
            .list_resources(Some(PaginatedRequestParams::default().with_cursor(cursor)))
            .await?;
        checked_extend(&mut items, result.resources, "resources")?;
        let Some(next) = result.next_cursor else {
            return Ok(items);
        };
        if !seen.insert(next.clone()) {
            bail!("resources/list repeated cursor `{next}`");
        }
        cursor = Some(next);
    }
    bail!("resources/list exceeded the {MAX_PAGES} page limit")
}

async fn list_resource_templates(peer: &Peer<RoleClient>) -> Result<Vec<ResourceTemplate>> {
    let mut items = Vec::new();
    let mut cursor = None;
    let mut seen = BTreeSet::new();
    for _ in 0..MAX_PAGES {
        let result = peer
            .list_resource_templates(Some(PaginatedRequestParams::default().with_cursor(cursor)))
            .await?;
        checked_extend(&mut items, result.resource_templates, "resource templates")?;
        let Some(next) = result.next_cursor else {
            return Ok(items);
        };
        if !seen.insert(next.clone()) {
            bail!("resources/templates/list repeated cursor `{next}`");
        }
        cursor = Some(next);
    }
    bail!("resources/templates/list exceeded the {MAX_PAGES} page limit")
}

fn checked_extend<T>(target: &mut Vec<T>, source: Vec<T>, kind: &str) -> Result<()> {
    if target.len().saturating_add(source.len()) > MAX_CATALOG_ITEMS {
        bail!("MCP server advertised more than {MAX_CATALOG_ITEMS} {kind}");
    }
    target.extend(source);
    Ok(())
}

fn normalize_server_identity(info: &rmcp::model::Implementation) -> McpServerIdentity {
    McpServerIdentity {
        name: bounded_text(&info.name),
        version: bounded_text(&info.version),
        title: info.title.as_deref().map(bounded_text),
        description: info.description.as_deref().map(bounded_text),
        website_url: info.website_url.as_deref().map(public_uri),
    }
}

fn normalize_capabilities(info: &ServerPeerInfo) -> McpServerCapabilities {
    let capabilities = &info.capabilities;
    let mut extensions = capabilities
        .extensions
        .as_ref()
        .map(|extensions| extensions.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    extensions.sort();
    McpServerCapabilities {
        tools: capabilities
            .tools
            .as_ref()
            .map(|capability| McpListCapability {
                list_changed: capability.list_changed,
            }),
        prompts: capabilities
            .prompts
            .as_ref()
            .map(|capability| McpListCapability {
                list_changed: capability.list_changed,
            }),
        resources: capabilities
            .resources
            .as_ref()
            .map(|capability| McpResourceCapability {
                list_changed: capability.list_changed,
                subscribe: capability.subscribe,
            }),
        logging: capabilities.logging.is_some(),
        completions: capabilities.completions.is_some(),
        experimental: capabilities.experimental.is_some(),
        extensions,
    }
}

fn normalize_tools(
    items: Vec<Tool>,
    registration: &McpRegistration,
) -> Result<Vec<McpRuntimeTool>> {
    items
        .into_iter()
        .map(|tool| {
            let enabled_by_registration = (registration.tool_policy.include.is_empty()
                || registration
                    .tool_policy
                    .include
                    .iter()
                    .any(|name| name == tool.name.as_ref()))
                && !registration
                    .tool_policy
                    .exclude
                    .iter()
                    .any(|name| name == tool.name.as_ref());
            let approval = registration
                .tool_policy
                .approval_overrides
                .get(tool.name.as_ref())
                .cloned()
                .or_else(|| registration.tool_policy.default_approval.clone());
            let annotations = tool.annotations.map(|annotations| McpToolAnnotations {
                title: annotations.title.map(|value| bounded_text(&value)),
                read_only: annotations.read_only_hint,
                destructive: annotations.destructive_hint,
                idempotent: annotations.idempotent_hint,
                open_world: annotations.open_world_hint,
            });
            Ok(McpRuntimeTool {
                name: bounded_text(tool.name.as_ref()),
                title: tool.title.as_deref().map(bounded_text),
                description: tool.description.as_deref().map(bounded_text),
                input_schema: bounded_json(serde_json::to_value(tool.input_schema.as_ref())?),
                output_schema: tool
                    .output_schema
                    .as_deref()
                    .map(serde_json::to_value)
                    .transpose()?
                    .map(bounded_json),
                annotations,
                enabled_by_registration,
                approval,
                meta_fields: meta_fields(tool.meta.as_ref()),
            })
        })
        .collect()
}

fn normalize_prompt(prompt: Prompt) -> McpRuntimePrompt {
    McpRuntimePrompt {
        name: bounded_text(&prompt.name),
        title: prompt.title.as_deref().map(bounded_text),
        description: prompt.description.as_deref().map(bounded_text),
        arguments: prompt
            .arguments
            .unwrap_or_default()
            .into_iter()
            .map(|argument| McpPromptArgument {
                name: bounded_text(&argument.name),
                title: argument.title.as_deref().map(bounded_text),
                description: argument.description.as_deref().map(bounded_text),
                required: argument.required,
            })
            .collect(),
        meta_fields: meta_fields(prompt.meta.as_ref()),
    }
}

fn normalize_resource(resource: &Resource) -> McpRuntimeResource {
    McpRuntimeResource {
        uri: public_uri(&resource.uri),
        name: bounded_text(&resource.name),
        title: resource.title.as_deref().map(bounded_text),
        description: resource.description.as_deref().map(bounded_text),
        mime_type: resource.mime_type.as_deref().map(bounded_text),
        size: resource.size,
        meta_fields: meta_fields(resource.meta.as_ref()),
    }
}

fn normalize_resource_template(template: &ResourceTemplate) -> McpRuntimeResourceTemplate {
    McpRuntimeResourceTemplate {
        uri_template: bounded_text(&template.uri_template),
        name: bounded_text(&template.name),
        title: template.title.as_deref().map(bounded_text),
        description: template.description.as_deref().map(bounded_text),
        mime_type: template.mime_type.as_deref().map(bounded_text),
        meta_fields: meta_fields(template.meta.as_ref()),
    }
}

fn meta_fields(meta: Option<&MetaObject>) -> Vec<String> {
    let mut fields = meta
        .map(|meta| meta.0.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    fields.sort();
    fields
}

fn bounded_json(value: serde_json::Value) -> serde_json::Value {
    let bytes = serde_json::to_vec(&value).map_or(usize::MAX, |bytes| bytes.len());
    if bytes <= MAX_SCHEMA_BYTES {
        value
    } else {
        serde_json::json!({
            "truncated": true,
            "original_bytes": bytes,
            "limit_bytes": MAX_SCHEMA_BYTES,
        })
    }
}

fn bounded_text(value: &str) -> String {
    value.chars().take(MAX_TEXT_CHARS).collect()
}

fn public_uri(value: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(value) else {
        return bounded_text(value);
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return bounded_text(value);
    }
    let query_names = parsed
        .query_pairs()
        .map(|(name, _)| name.into_owned())
        .collect::<Vec<_>>();
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
    bounded_text(&public)
}

fn parse_header_name(name: &str) -> Result<HeaderName> {
    HeaderName::from_bytes(name.as_bytes())
        .with_context(|| format!("configured MCP header name `{name}` is invalid"))
}

fn parse_header_value(name: &str, value: &str) -> Result<HeaderValue> {
    HeaderValue::from_str(value)
        .with_context(|| format!("configured value for MCP header `{name}` is invalid"))
}

fn resolve_template(
    template: &str,
    _environments: &[String],
    variables: &HashMapLike,
    target: &str,
) -> Result<String> {
    resolve_config_value(template, variables, target)
}

type HashMapLike = std::collections::BTreeMap<String, String>;

fn resolve_config_value(value: &str, variables: &HashMapLike, target: &str) -> Result<String> {
    let environments = configuration_references(value);
    let mut resolved = value.to_owned();
    for (expression, environment, default) in environments {
        let replacement = variables
            .get(&environment)
            .cloned()
            .or_else(|| std::env::var(&environment).ok())
            .or(default)
            .with_context(|| {
                format!(
                    "required environment variable `{environment}` for MCP {target} is not set or is not Unicode"
                )
            })?;
        resolved = resolved.replace(&expression, &replacement);
    }
    Ok(resolved)
}

fn resolve_executable(
    command: &str,
    cwd: Option<&std::path::Path>,
    variables: &HashMapLike,
) -> Result<String> {
    let path = std::path::Path::new(command);
    if path.is_absolute() || path.components().count() == 1 {
        return Ok(command.to_owned());
    }
    let cwd = cwd.map_or_else(
        || Ok(std::env::current_dir()?),
        |cwd| {
            let resolved = resolve_config_value(&cwd.to_string_lossy(), variables, "cwd")?;
            Ok::<_, anyhow::Error>(std::path::PathBuf::from(resolved))
        },
    )?;
    Ok(cwd.join(path).display().to_string())
}

fn configuration_references(value: &str) -> Vec<(String, String, Option<String>)> {
    let mut references = Vec::new();
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'{' && value[index..].starts_with("{env:") {
            let start = index;
            let name_start = index + 5;
            if let Some(relative_end) = value[name_start..].find('}') {
                let end = name_start + relative_end;
                let name = &value[name_start..end];
                if valid_environment_name(name) {
                    references.push((value[start..=end].to_owned(), name.to_owned(), None));
                }
                index = end + 1;
                continue;
            }
        }
        if bytes[index] != b'$' {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        if index >= bytes.len() {
            break;
        }
        if bytes[index] == b'{' {
            let body_start = index + 1;
            let Some(relative_end) = value[body_start..].find('}') else {
                break;
            };
            let end = body_start + relative_end;
            let body = &value[body_start..end];
            let (name, default) = body
                .split_once(":-")
                .map_or((body, None), |(name, default)| {
                    (name, Some(default.to_owned()))
                });
            if valid_environment_name(name) {
                references.push((value[start..=end].to_owned(), name.to_owned(), default));
            }
            index = end + 1;
            continue;
        }
        let name_start = index;
        while index < bytes.len() && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
        {
            index += 1;
        }
        let name = &value[name_start..index];
        if valid_environment_name(name) {
            references.push((value[start..index].to_owned(), name.to_owned(), None));
        }
    }
    references
}

fn valid_environment_name(name: &str) -> bool {
    !name.is_empty()
        && (name.as_bytes()[0].is_ascii_alphabetic() || name.as_bytes()[0] == b'_')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn failed(started: Instant, error: String) -> McpProbe {
    McpProbe {
        status: McpProbeStatus::Failed,
        duration_ms: elapsed_millis(started),
        protocol_version: None,
        server: None,
        capabilities: None,
        instructions: None,
        tools: Vec::new(),
        prompts: Vec::new(),
        resources: Vec::new(),
        resource_templates: Vec::new(),
        warnings: Vec::new(),
        error: Some(error),
    }
}

fn elapsed_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn sanitize_error(
    error: &str,
    connection: &McpConnection,
    registration: &McpRegistration,
) -> String {
    let mut sanitized = error.to_owned();
    match connection {
        McpConnection::Stdio {
            command,
            args,
            environment,
            environment_sources,
            environment_templates,
            variables,
            cwd,
            ..
        } => {
            for value in environment.values() {
                replace_secret(&mut sanitized, value);
            }
            for source in environment_sources.values() {
                if let Ok(value) = std::env::var(source) {
                    replace_secret(&mut sanitized, &value);
                }
            }
            for (template, environments) in environment_templates.values() {
                if let Ok(value) =
                    resolve_template(template, environments, variables, "environment")
                {
                    replace_secret(&mut sanitized, &value);
                }
            }
            if let Some(public) = &registration.command {
                replace_resolved_config_value(
                    &mut sanitized,
                    command,
                    variables,
                    public,
                    "command",
                );
            }
            for (raw, public) in args.iter().zip(&registration.args) {
                replace_resolved_config_value(&mut sanitized, raw, variables, public, "argument");
            }
            if let (Some(raw), Some(public)) = (cwd, &registration.cwd) {
                replace_resolved_config_value(
                    &mut sanitized,
                    &raw.to_string_lossy(),
                    variables,
                    &public.to_string_lossy(),
                    "cwd",
                );
            }
        }
        McpConnection::Http {
            url,
            headers,
            environment_headers,
            environment_templates,
            variables,
            bearer_token_environment,
            ..
        } => {
            if let Some(public) = &registration.url {
                replace_resolved_config_value(&mut sanitized, url, variables, public, "url");
            }
            for value in headers.values() {
                replace_secret(&mut sanitized, value);
            }
            for environment in environment_headers.values() {
                if let Ok(value) = std::env::var(environment) {
                    replace_secret(&mut sanitized, &value);
                }
            }
            for (template, environments) in environment_templates.values() {
                if let Ok(value) = resolve_template(template, environments, variables, "header") {
                    replace_secret(&mut sanitized, &value);
                }
            }
            if let Some(environment) = bearer_token_environment
                && let Ok(value) = std::env::var(environment)
            {
                replace_secret(&mut sanitized, &value);
            }
        }
        McpConnection::Unsupported { .. } => {}
    }
    sanitized.chars().take(MAX_ERROR_CHARS).collect()
}

fn replace_secret(target: &mut String, secret: &str) {
    if !secret.is_empty() {
        *target = target.replace(secret, "<redacted>");
    }
}

fn replace_resolved_config_value(
    target: &mut String,
    raw: &str,
    variables: &HashMapLike,
    public: &str,
    kind: &str,
) {
    if let Ok(resolved) = resolve_config_value(raw, variables, kind) {
        *target = target.replace(&resolved, public);
    }
    *target = target.replace(raw, public);
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    use anyhow::Result;
    use rmcp::model::{
        CallToolRequestParams, CallToolResponse, Implementation, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool, ToolAnnotations,
    };
    use rmcp::service::RequestContext;
    use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
    use serde_json::{Map, json};

    use super::collect;
    use super::sanitize_error;
    use crate::mcp::adapter::McpConnection;
    use crate::mcp::{McpRegistration, McpSourceFormat, McpTimeouts, McpToolPolicy, McpTransport};

    struct CatalogServer {
        tool_calls: Arc<AtomicUsize>,
    }

    impl ServerHandler for CatalogServer {
        fn get_info(&self) -> ServerInfo {
            ServerInfo::new(
                ServerCapabilities::builder()
                    .enable_tools()
                    .enable_tool_list_changed()
                    .build(),
            )
            .with_server_info(
                Implementation::new("fixture-server", "1.2.3")
                    .with_title("Fixture MCP")
                    .with_description("Test metadata"),
            )
            .with_instructions("Use search for read-only lookups")
        }

        async fn list_tools(
            &self,
            _request: Option<PaginatedRequestParams>,
            _context: RequestContext<RoleServer>,
        ) -> Result<ListToolsResult, ErrorData> {
            let mut schema = Map::new();
            schema.insert("type".to_owned(), json!("object"));
            let mut tool = Tool::new("search", "Search documentation", schema)
                .with_title("Documentation search");
            tool.annotations = Some(
                ToolAnnotations::new()
                    .read_only(true)
                    .destructive(false)
                    .idempotent(true)
                    .open_world(false),
            );
            Ok(ListToolsResult::with_all_items(vec![tool]))
        }

        async fn call_tool(
            &self,
            _request: CallToolRequestParams,
            _context: RequestContext<RoleServer>,
        ) -> Result<CallToolResponse, ErrorData> {
            self.tool_calls.fetch_add(1, Ordering::SeqCst);
            Err(ErrorData::internal_error("tool calls are forbidden", None))
        }
    }

    #[test]
    fn live_catalog_collects_tool_metadata_without_calling_tools() -> Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            let calls = Arc::new(AtomicUsize::new(0));
            let (server_transport, client_transport) = tokio::io::duplex(16 * 1_024);
            let server_calls = calls.clone();
            let server_task = tokio::spawn(async move {
                let server = CatalogServer {
                    tool_calls: server_calls,
                }
                .serve(server_transport)
                .await
                .expect("serve fixture");
                server.waiting().await.expect("wait for fixture");
            });
            let mut client = ().serve(client_transport).await?;
            let registration = registration();
            let probe = collect(client.peer(), &registration, Instant::now()).await?;
            assert_eq!(probe.status, crate::mcp::McpProbeStatus::Success);
            assert_eq!(
                probe.server.as_ref().map(|server| server.name.as_str()),
                Some("fixture-server")
            );
            assert_eq!(probe.tools.len(), 1);
            assert_eq!(probe.tools[0].name, "search");
            assert_eq!(
                probe.tools[0].description.as_deref(),
                Some("Search documentation")
            );
            assert!(probe.tools[0].enabled_by_registration);
            assert_eq!(
                probe.tools[0]
                    .annotations
                    .as_ref()
                    .and_then(|annotations| annotations.read_only),
                Some(true)
            );
            assert_eq!(calls.load(Ordering::SeqCst), 0);
            client.close().await?;
            server_task.await?;
            Ok::<_, anyhow::Error>(())
        })?;
        Ok(())
    }

    #[test]
    fn probe_errors_redact_connection_credentials() {
        let registration = McpRegistration {
            url: Some("https://example.test/mcp?token=<redacted>".to_owned()),
            transport: McpTransport::StreamableHttp,
            command: None,
            ..registration()
        };
        let connection = McpConnection::Http {
            url: "https://alice:${MCP_PASSWORD}@example.test/mcp?token=${MCP_TOKEN}".to_owned(),
            headers: BTreeMap::from([("Authorization".to_owned(), "header-secret".to_owned())]),
            environment_headers: BTreeMap::new(),
            environment_templates: BTreeMap::new(),
            variables: BTreeMap::from([
                ("MCP_PASSWORD".to_owned(), "password".to_owned()),
                ("MCP_TOKEN".to_owned(), "query-secret".to_owned()),
            ]),
            bearer_token_environment: None,
            unresolved_values: Vec::new(),
        };
        let error = sanitize_error(
            "request https://alice:password@example.test/mcp?token=query-secret failed with header-secret",
            &connection,
            &registration,
        );
        assert!(error.contains("https://example.test/mcp?token=<redacted>"));
        for secret in ["alice", "password", "query-secret", "header-secret"] {
            assert!(!error.contains(secret));
        }
    }

    fn registration() -> McpRegistration {
        McpRegistration {
            selector: "codex:user:fixture".to_owned(),
            name: "fixture".to_owned(),
            provider: "codex".to_owned(),
            scope: "user".to_owned(),
            source: "/tmp/config.toml".into(),
            source_format: McpSourceFormat::Toml,
            transport: McpTransport::Stdio,
            enabled: true,
            valid: true,
            display_name: None,
            description: None,
            command: Some("fixture".to_owned()),
            args: Vec::new(),
            url: None,
            cwd: None,
            timeouts: McpTimeouts::default(),
            authentication: Vec::new(),
            environment: Vec::new(),
            headers: Vec::new(),
            tool_policy: McpToolPolicy::default(),
            options: BTreeMap::new(),
            extra_fields: Vec::new(),
            warnings: Vec::new(),
        }
    }
}
