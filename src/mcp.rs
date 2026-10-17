use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

mod adapter;
mod probe;

use adapter::discover_mcp_servers;

pub fn ensure_source_editable(registration: &McpRegistration) -> Result<()> {
    adapter::ensure_source_editable(registration)
}

pub fn ensure_config_deletable(registration: &McpRegistration) -> Result<()> {
    adapter::ensure_config_deletable(registration)
}

/// Transport or host mechanism used to expose an MCP server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    Stdio,
    StreamableHttp,
    Sse,
    Builtin,
    Platform,
    Frontend,
    InlinePython,
    Unknown,
}

impl McpTransport {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::StreamableHttp => "streamable_http",
            Self::Sse => "sse",
            Self::Builtin => "builtin",
            Self::Platform => "platform",
            Self::Frontend => "frontend",
            Self::InlinePython => "inline_python",
            Self::Unknown => "unknown",
        }
    }
}

/// On-disk syntax used by the source registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpSourceFormat {
    Toml,
    Json,
    Jsonc,
    Yaml,
}

/// Configuration-time and request-time limits normalized to milliseconds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpTimeouts {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub startup_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_ms: Option<u64>,
}

/// Where a secret-bearing environment or header value is resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpValueSource {
    Literal,
    Environment,
    Forwarded,
    RemoteEnvironment,
    DynamicCommand,
    ProviderCredentialStore,
    Unknown,
}

/// A named binding with its value deliberately omitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpValueBinding {
    pub name: String,
    pub source: McpValueSource,
    pub sensitive: bool,
}

/// Authentication metadata that never contains credential material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpAuthentication {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

/// Static tool filtering and approval metadata from the client configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpToolPolicy {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_approval: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub approval_overrides: BTreeMap<String, String>,
}

/// One MCP registration from one client and configuration scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpRegistration {
    pub selector: String,
    pub name: String,
    pub provider: String,
    pub scope: String,
    pub source: PathBuf,
    pub source_format: McpSourceFormat,
    pub transport: McpTransport,
    pub enabled: bool,
    pub valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    pub timeouts: McpTimeouts,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authentication: Vec<McpAuthentication>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub environment: Vec<McpValueBinding>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<McpValueBinding>,
    pub tool_policy: McpToolPolicy,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub options: BTreeMap<String, serde_json::Value>,
    /// Configuration keys that were present but have no normalized semantic.
    /// Values are deliberately omitted so unknown secret-bearing fields cannot
    /// leak through machine-readable output.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// A partial update to the non-secret connection fields of one MCP registration.
///
/// An outer [`Option`] distinguishes an unchanged field from a requested
/// update. For nullable fields, `Some(None)` removes the field from the native
/// configuration while `Some(Some(value))` replaces it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpConfigPatch {
    pub enabled: Option<bool>,
    pub command: Option<Option<String>>,
    pub args: Option<Vec<String>>,
    pub url: Option<Option<String>>,
    pub cwd: Option<Option<PathBuf>>,
}

impl McpConfigPatch {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.enabled.is_none()
            && self.command.is_none()
            && self.args.is_none()
            && self.url.is_none()
            && self.cwd.is_none()
    }
}

/// Runtime server identity returned by the MCP initialization handshake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerIdentity {
    pub name: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub website_url: Option<String>,
}

/// One list-style server capability and its change-notification support.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpListCapability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_changed: Option<bool>,
}

/// Resource capability details advertised during initialization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpResourceCapability {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_changed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscribe: Option<bool>,
}

/// Provider-neutral view of server-advertised MCP capabilities.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<McpListCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompts: Option<McpListCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resources: Option<McpResourceCapability>,
    pub logging: bool,
    pub completions: bool,
    pub experimental: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
}

/// Safety hints supplied by an MCP server for a runtime tool.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpToolAnnotations {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub destructive: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotent: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_world: Option<bool>,
}

/// Tool metadata returned by `tools/list`; tools are never called by a probe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpRuntimeTool {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub input_schema: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<McpToolAnnotations>,
    pub enabled_by_registration: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub meta_fields: Vec<String>,
}

/// Argument metadata for a runtime prompt template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpPromptArgument {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<bool>,
}

/// Prompt metadata returned by `prompts/list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpRuntimePrompt {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arguments: Vec<McpPromptArgument>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub meta_fields: Vec<String>,
}

/// Concrete resource metadata returned by `resources/list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpRuntimeResource {
    pub uri: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub meta_fields: Vec<String>,
}

/// Resource-template metadata returned by `resources/templates/list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpRuntimeResourceTemplate {
    pub uri_template: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub meta_fields: Vec<String>,
}

/// Closed outcome set for an explicit live MCP metadata probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpProbeStatus {
    Success,
    Partial,
    Failed,
    Refused,
    Unsupported,
}

impl McpProbeStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Partial => "partial",
            Self::Failed => "failed",
            Self::Refused => "refused",
            Self::Unsupported => "unsupported",
        }
    }
}

impl fmt::Display for McpProbeStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Runtime metadata is absent until the user explicitly requests a probe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpProbe {
    pub status: McpProbeStatus,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<McpServerIdentity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<McpServerCapabilities>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<McpRuntimeTool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompts: Vec<McpRuntimePrompt>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<McpRuntimeResource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resource_templates: Vec<McpRuntimeResourceTemplate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl McpProbe {
    fn decision(status: McpProbeStatus, error: impl Into<String>) -> Self {
        Self {
            status,
            duration_ms: 0,
            protocol_version: None,
            server: None,
            capabilities: None,
            instructions: None,
            tools: Vec::new(),
            prompts: Vec::new(),
            resources: Vec::new(),
            resource_templates: Vec::new(),
            warnings: Vec::new(),
            error: Some(error.into()),
        }
    }
}

/// Full inspection result for one registration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpDetail {
    #[serde(flatten)]
    pub registration: McpRegistration,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub probe: Option<McpProbe>,
}

/// Provider-neutral catalog of MCP registrations.
#[derive(Default)]
pub struct McpCatalog {
    registrations: Vec<McpRegistration>,
    connections: Vec<adapter::McpConnection>,
    home: Option<PathBuf>,
    workspace: Option<PathBuf>,
}

impl fmt::Debug for McpCatalog {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("McpCatalog")
            .field("registrations", &self.registrations)
            .finish_non_exhaustive()
    }
}

impl McpCatalog {
    /// Discover MCP registrations from supported local agent configurations.
    ///
    /// # Errors
    ///
    /// Returns an error when an existing configuration source cannot be read
    /// or parsed. Missing sources are ignored.
    pub fn scan(home: Option<&Path>, workspace: Option<&Path>) -> Result<Self> {
        let mut discovered = discover_mcp_servers(home, workspace)?;
        discovered.sort_by(|left, right| {
            left.registration
                .provider
                .cmp(&right.registration.provider)
                .then_with(|| left.registration.scope.cmp(&right.registration.scope))
                .then_with(|| left.registration.name.cmp(&right.registration.name))
                .then_with(|| left.registration.source.cmp(&right.registration.source))
        });
        let (registrations, connections) = discovered
            .into_iter()
            .map(|entry| (entry.registration, entry.connection))
            .unzip();
        Ok(Self {
            registrations,
            connections,
            home: home.map(Path::to_path_buf),
            workspace: workspace.map(Path::to_path_buf),
        })
    }

    #[must_use]
    pub fn registrations(&self) -> &[McpRegistration] {
        &self.registrations
    }

    /// Resolve the current one-based source line for a registration.
    ///
    /// Returns line 1 when a valid but unusual native spelling cannot be
    /// located precisely, so opening the source remains available.
    ///
    /// # Errors
    ///
    /// Returns an error when the source cannot be read.
    pub fn source_line(&self, registration: &McpRegistration) -> Result<usize> {
        adapter::source_line(registration, self.workspace.as_deref())
    }

    /// Permanently remove one registration from its writable native source.
    ///
    /// # Errors
    ///
    /// Returns an error when the registration is stale, its source is
    /// read-only or cannot preserve comments, or the atomic update fails.
    pub fn delete_registration(&self, registration: &McpRegistration) -> Result<()> {
        let current = Self::scan(self.home.as_deref(), self.workspace.as_deref())?;
        let index = current.resolve_identity_index(registration)?;
        let current_registration = &current.registrations[index];
        adapter::delete_config(current_registration, self.workspace.as_deref())?;
        let refreshed = Self::scan(self.home.as_deref(), self.workspace.as_deref())?;
        if refreshed.registrations.iter().any(|candidate| {
            candidate.selector == current_registration.selector
                && candidate.source == current_registration.source
        }) {
            bail!(
                "MCP registration `{}` is still present after deletion",
                current_registration.selector
            );
        }
        Ok(())
    }

    /// Rediscover and return a filtered snapshot of the current registrations.
    ///
    /// # Errors
    ///
    /// Returns an error when a configuration cannot be read or parsed, or a
    /// filter value is unsupported.
    pub fn refresh_selection(
        &self,
        provider: Option<&str>,
        scope: Option<&str>,
        source: Option<&str>,
    ) -> Result<Vec<McpRegistration>> {
        let current = Self::scan(self.home.as_deref(), self.workspace.as_deref())?;
        Ok(current
            .select(provider, scope, source)?
            .into_iter()
            .cloned()
            .collect())
    }

    /// Atomically update basic connection fields in one registration's native
    /// configuration and return the freshly rediscovered registration.
    ///
    /// Secret-bearing environment and header values are never part of the
    /// patch, and unmodified native fields are preserved.
    ///
    /// # Errors
    ///
    /// Returns an error when the registration is stale, its source is read-only
    /// or unsupported, the patch is invalid, or the updated catalog cannot be
    /// rediscovered.
    pub fn update_basic_config(
        &self,
        registration: &McpRegistration,
        patch: &McpConfigPatch,
    ) -> Result<McpRegistration> {
        let current = Self::scan(self.home.as_deref(), self.workspace.as_deref())?;
        let current_index = current.resolve_identity_index(registration)?;
        let current_registration = &current.registrations[current_index];
        if patch.is_empty() {
            return Ok(current_registration.clone());
        }

        adapter::update_basic_config(current_registration, patch, self.workspace.as_deref())?;
        let refreshed = Self::scan(self.home.as_deref(), self.workspace.as_deref())?;
        let index = refreshed.resolve_identity_index(current_registration)?;
        Ok(refreshed.registrations[index].clone())
    }

    /// Select registrations using normalized provider, scope, and source filters.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported provider or scope values.
    pub fn select(
        &self,
        provider: Option<&str>,
        scope: Option<&str>,
        source: Option<&str>,
    ) -> Result<Vec<&McpRegistration>> {
        validate_filter("provider", provider, MCP_PROVIDERS)?;
        validate_filter("scope", scope, MCP_SCOPES)?;
        let source = source.map(Path::new);
        Ok(self
            .registrations
            .iter()
            .filter(|registration| {
                provider.is_none_or(|value| registration.provider == value)
                    && scope.is_none_or(|value| registration.scope == value)
                    && source.is_none_or(|value| {
                        registration.source == value || registration.source.ends_with(value)
                    })
            })
            .collect())
    }

    /// Inspect one registration without contacting or launching its server.
    ///
    /// The name may be a bare registration name or a stable
    /// `provider:scope:name` selector. A bare name must resolve uniquely after
    /// filters are applied.
    ///
    /// # Errors
    ///
    /// Returns an error when filters are unsupported, no registration matches,
    /// or the target remains ambiguous.
    pub fn inspect(
        &self,
        name: &str,
        provider: Option<&str>,
        scope: Option<&str>,
        source: Option<&str>,
    ) -> Result<McpDetail> {
        let index = self.resolve_index(name, provider, scope, source)?;
        Ok(McpDetail {
            registration: self.registrations[index].clone(),
            probe: None,
        })
    }

    /// Inspect one registration and explicitly connect to discover live MCP
    /// protocol metadata. This performs initialization and list operations only;
    /// it never invokes a tool, reads a resource, or renders a prompt.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid filters, ambiguous targets, or timeout
    /// values outside 1 through 300 seconds. Connection/protocol failures are
    /// represented in [`McpProbe::status`] so JSON output remains inspectable.
    pub fn inspect_with_probe(
        &self,
        name: &str,
        provider: Option<&str>,
        scope: Option<&str>,
        source: Option<&str>,
        timeout_seconds: u64,
    ) -> Result<McpDetail> {
        if !(1..=300).contains(&timeout_seconds) {
            bail!("MCP probe timeout must be between 1 and 300 seconds");
        }
        let index = self.resolve_index(name, provider, scope, source)?;
        Ok(self.probe_index(index, timeout_seconds))
    }

    pub(crate) fn inspect_current_registration_with_probe(
        &self,
        registration: &McpRegistration,
        timeout_seconds: u64,
    ) -> Result<McpDetail> {
        if !(1..=300).contains(&timeout_seconds) {
            bail!("MCP probe timeout must be between 1 and 300 seconds");
        }
        let refreshed = Self::scan(self.home.as_deref(), self.workspace.as_deref())?;
        let index = refreshed.resolve_identity_index(registration)?;
        Ok(refreshed.probe_index(index, timeout_seconds))
    }

    fn probe_index(&self, index: usize, timeout_seconds: u64) -> McpDetail {
        let registration = self.registrations[index].clone();
        let probe = if !registration.enabled {
            McpProbe::decision(
                McpProbeStatus::Refused,
                "registration is disabled; enable it before requesting a live probe",
            )
        } else if !registration.valid {
            McpProbe::decision(
                McpProbeStatus::Refused,
                "registration is invalid or transport selection is ambiguous",
            )
        } else if let adapter::McpConnection::Unsupported { reason } = &self.connections[index] {
            McpProbe::decision(McpProbeStatus::Unsupported, reason.clone())
        } else {
            probe::run(
                &self.connections[index],
                &registration,
                std::time::Duration::from_secs(timeout_seconds),
            )
        };
        McpDetail {
            registration,
            probe: Some(probe),
        }
    }

    fn resolve_index(
        &self,
        name: &str,
        provider: Option<&str>,
        scope: Option<&str>,
        source: Option<&str>,
    ) -> Result<usize> {
        validate_filter("provider", provider, MCP_PROVIDERS)?;
        validate_filter("scope", scope, MCP_SCOPES)?;
        let source = source.map(Path::new);
        let candidates: Vec<usize> = self
            .registrations
            .iter()
            .enumerate()
            .filter(|(_, registration)| {
                (registration.name == name || registration.selector == name)
                    && provider.is_none_or(|value| registration.provider == value)
                    && scope.is_none_or(|value| registration.scope == value)
                    && source.is_none_or(|value| {
                        registration.source == value || registration.source.ends_with(value)
                    })
            })
            .map(|(index, _)| index)
            .collect();
        match candidates.as_slice() {
            [] => bail!("MCP registration `{name}` was not found with the requested filters"),
            [index] => Ok(*index),
            candidates => {
                let matches = candidates
                    .iter()
                    .map(|index| {
                        let registration = &self.registrations[*index];
                        format!(
                            "{} ({})",
                            registration.selector,
                            registration.source.display()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                bail!(
                    "ambiguous MCP registration `{name}`; matches: {matches}; use a full selector or --provider/--scope/--source"
                )
            }
        }
    }

    fn resolve_identity_index(&self, registration: &McpRegistration) -> Result<usize> {
        let matches = self
            .registrations
            .iter()
            .enumerate()
            .filter(|(_, candidate)| {
                candidate.selector == registration.selector
                    && candidate.source == registration.source
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [index] => Ok(*index),
            [] => bail!(
                "MCP registration `{}` is no longer present in {}",
                registration.selector,
                registration.source.display()
            ),
            _ => bail!(
                "MCP registration `{}` is ambiguous in {}",
                registration.selector,
                registration.source.display()
            ),
        }
    }
}

const MCP_PROVIDERS: &[&str] = &[
    "claude", "codex", "cursor", "gemini", "goose", "omp", "opencode", "pi",
];
const MCP_SCOPES: &[&str] = &[
    "user", "local", "project", "plugin", "profile", "managed", "shared",
];

fn validate_filter(kind: &str, value: Option<&str>, supported: &[&str]) -> Result<()> {
    if let Some(value) = value
        && !supported.contains(&value)
    {
        bail!(
            "unsupported MCP {kind} `{value}`; available {kind}s: {}",
            supported.join(", ")
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
