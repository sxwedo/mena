use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

mod adapter;
mod probe;

use adapter::discover_mcp_servers;

pub fn ensure_basic_config_editable(registration: &McpRegistration) -> Result<()> {
    adapter::ensure_basic_config_editable(registration)
}

pub fn basic_config_can_toggle_enabled(registration: &McpRegistration) -> bool {
    adapter::basic_config_can_toggle_enabled(registration)
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
mod tests {
    use std::fs;

    use anyhow::Result;
    use tempfile::tempdir;

    use super::{McpCatalog, McpTransport, McpValueSource};

    #[test]
    fn basic_edit_updates_only_the_selected_codex_registration() -> Result<()> {
        let home = tempdir()?;
        fs::create_dir_all(home.path().join(".codex"))?;
        let path = home.path().join(".codex/config.toml");
        fs::write(
            &path,
            r#"# keep this file comment
[features]
experimental = true

[mcp_servers.docs]
# keep this server comment
command = "npx"
args = ["-y", "@example/docs"]
cwd = "/old/workspace"
enabled = true
env = { API_TOKEN = "keep-secret" }

[mcp_servers.other]
command = "other-server"
"#,
        )?;

        let catalog = McpCatalog::scan(Some(home.path()), None)?;
        let registration = catalog
            .registrations()
            .iter()
            .find(|registration| registration.name == "docs")
            .expect("docs registration");
        let updated = catalog.update_basic_config(
            registration,
            &super::McpConfigPatch {
                command: Some(Some("bunx".to_owned())),
                args: Some(vec!["@example/docs@latest".to_owned()]),
                cwd: Some(None),
                ..super::McpConfigPatch::default()
            },
        )?;

        assert_eq!(updated.command.as_deref(), Some("bunx"));
        assert_eq!(updated.args, ["@example/docs@latest"]);
        assert_eq!(updated.cwd, None);

        let content = fs::read_to_string(path)?;
        assert!(content.contains("# keep this file comment"));
        assert!(content.contains("# keep this server comment"));
        assert!(content.contains("API_TOKEN = \"keep-secret\""));
        assert!(content.contains("[mcp_servers.other]"));
        assert!(content.contains("command = \"other-server\""));
        Ok(())
    }

    #[test]
    fn empty_basic_edit_patch_rediscovers_external_changes() -> Result<()> {
        let home = tempdir()?;
        fs::create_dir_all(home.path().join(".codex"))?;
        let path = home.path().join(".codex/config.toml");
        fs::write(
            &path,
            r#"[mcp_servers.docs]
command = "old-server"
"#,
        )?;

        let catalog = McpCatalog::scan(Some(home.path()), None)?;
        let registration = catalog
            .registrations()
            .iter()
            .find(|registration| registration.name == "docs")
            .expect("docs registration");
        fs::write(
            &path,
            r#"[mcp_servers.docs]
command = "new-server"
"#,
        )?;

        let refreshed =
            catalog.update_basic_config(registration, &super::McpConfigPatch::default())?;

        assert_eq!(refreshed.command.as_deref(), Some("new-server"));
        Ok(())
    }

    #[test]
    fn basic_edit_preserves_unrelated_json_configuration() -> Result<()> {
        let home = tempdir()?;
        fs::create_dir_all(home.path().join(".cursor"))?;
        let path = home.path().join(".cursor/mcp.json");
        fs::write(
            &path,
            r#"{
  "theme": "keep-root-setting",
  "mcpServers": {
    "docs": {
      "command": "npx",
      "args": ["old"],
      "cwd": "/old/workspace",
      "enabled": true,
      "env": {"API_TOKEN": "keep-secret"},
      "custom": {"keep": true}
    },
    "other": {"command": "other-server"}
  }
}"#,
        )?;

        let catalog = McpCatalog::scan(Some(home.path()), None)?;
        let registration = catalog
            .registrations()
            .iter()
            .find(|registration| registration.selector == "cursor:user:docs")
            .expect("docs registration");
        let updated = catalog.update_basic_config(
            registration,
            &super::McpConfigPatch {
                command: Some(Some("bunx".to_owned())),
                args: Some(vec!["@example/docs@latest".to_owned()]),
                cwd: Some(None),
                ..super::McpConfigPatch::default()
            },
        )?;

        assert_eq!(updated.command.as_deref(), Some("bunx"));
        assert_eq!(updated.args, ["@example/docs@latest"]);
        assert_eq!(updated.cwd, None);

        let content: serde_json::Value = serde_json::from_str(&fs::read_to_string(path)?)?;
        assert_eq!(content["theme"], "keep-root-setting");
        assert_eq!(
            content["mcpServers"]["docs"]["env"]["API_TOKEN"],
            "keep-secret"
        );
        assert_eq!(content["mcpServers"]["docs"]["custom"]["keep"], true);
        assert_eq!(content["mcpServers"]["other"]["command"], "other-server");
        Ok(())
    }

    #[test]
    fn basic_edit_uses_opencode_native_command_array() -> Result<()> {
        let home = tempdir()?;
        fs::create_dir_all(home.path().join(".config/opencode"))?;
        let path = home.path().join(".config/opencode/opencode.json");
        fs::write(
            &path,
            r#"{
  "mcp": {
    "servers": {
      "docs": {
        "type": "local",
        "command": ["npx", "-y", "old-package"],
        "disabled": false,
        "environment": {"API_TOKEN": "keep-secret"}
      }
    }
  }
}"#,
        )?;

        let catalog = McpCatalog::scan(Some(home.path()), None)?;
        let registration = catalog
            .registrations()
            .iter()
            .find(|registration| registration.selector == "opencode:user:docs")
            .expect("docs registration");
        let updated = catalog.update_basic_config(
            registration,
            &super::McpConfigPatch {
                enabled: Some(false),
                command: Some(Some("bunx".to_owned())),
                args: Some(vec!["new-package".to_owned(), "--stdio".to_owned()]),
                ..super::McpConfigPatch::default()
            },
        )?;

        assert!(!updated.enabled);
        assert_eq!(updated.command.as_deref(), Some("bunx"));
        assert_eq!(updated.args, ["new-package", "--stdio"]);
        let content: serde_json::Value = serde_json::from_str(&fs::read_to_string(path)?)?;
        assert_eq!(
            content["mcp"]["servers"]["docs"]["command"],
            serde_json::json!(["bunx", "new-package", "--stdio"])
        );
        assert_eq!(content["mcp"]["servers"]["docs"]["disabled"], true);
        assert_eq!(
            content["mcp"]["servers"]["docs"]["environment"]["API_TOKEN"],
            "keep-secret"
        );
        Ok(())
    }

    #[test]
    fn basic_edit_targets_the_nearest_claude_local_project() -> Result<()> {
        let home = tempdir()?;
        let project = home.path().join("work/project");
        let nested = project.join("nested/current");
        fs::create_dir_all(&nested)?;
        let path = home.path().join(".claude.json");
        let source = serde_json::json!({
            "projects": {
                project.to_string_lossy(): {
                    "mcpServers": {
                        "docs": {"command": "parent-server"}
                    }
                },
                project.join("nested").to_string_lossy(): {
                    "mcpServers": {
                        "docs": {"command": "nested-server"}
                    }
                }
            }
        });
        fs::write(&path, serde_json::to_vec_pretty(&source)?)?;

        let catalog = McpCatalog::scan(Some(home.path()), Some(&nested))?;
        let registration = catalog
            .registrations()
            .iter()
            .find(|registration| registration.selector == "claude:local:docs")
            .expect("local docs registration");
        assert_eq!(registration.command.as_deref(), Some("nested-server"));

        let updated = catalog.update_basic_config(
            registration,
            &super::McpConfigPatch {
                command: Some(Some("updated-nested-server".to_owned())),
                ..super::McpConfigPatch::default()
            },
        )?;
        assert_eq!(updated.command.as_deref(), Some("updated-nested-server"));

        let content: serde_json::Value = serde_json::from_str(&fs::read_to_string(path)?)?;
        assert_eq!(
            content["projects"][project.to_string_lossy().as_ref()]["mcpServers"]["docs"]["command"],
            "parent-server"
        );
        assert_eq!(
            content["projects"][project.join("nested").to_string_lossy().as_ref()]["mcpServers"]["docs"]
                ["command"],
            "updated-nested-server"
        );
        Ok(())
    }

    #[test]
    fn basic_edit_updates_omp_native_enabled_lists() -> Result<()> {
        let home = tempdir()?;
        fs::create_dir_all(home.path().join(".omp/agent"))?;
        let path = home.path().join(".omp/agent/mcp.json");
        fs::write(
            &path,
            r#"{
  "mcpServers": {"docs": {"command": "docs-server"}},
  "enabledServers": ["docs", "other"],
  "disabledServers": ["legacy"]
}"#,
        )?;

        let catalog = McpCatalog::scan(Some(home.path()), None)?;
        let registration = catalog
            .registrations()
            .iter()
            .find(|registration| registration.selector == "omp:user:docs")
            .expect("OMP docs registration");
        let updated = catalog.update_basic_config(
            registration,
            &super::McpConfigPatch {
                enabled: Some(false),
                ..super::McpConfigPatch::default()
            },
        )?;

        assert!(!updated.enabled);
        let content: serde_json::Value = serde_json::from_str(&fs::read_to_string(path)?)?;
        assert_eq!(content["enabledServers"], serde_json::json!(["other"]));
        assert_eq!(
            content["disabledServers"],
            serde_json::json!(["legacy", "docs"])
        );
        Ok(())
    }

    #[test]
    fn basic_edit_updates_gemini_allowed_and_excluded_policy() -> Result<()> {
        let home = tempdir()?;
        fs::create_dir_all(home.path().join(".gemini"))?;
        let path = home.path().join(".gemini/settings.json");
        fs::write(
            &path,
            r#"{
  "mcpServers": {
    "docs": {"command": "docs-server"},
    "other": {"command": "other-server"}
  },
  "mcp": {
    "allowed": ["docs"],
    "excluded": []
  }
}"#,
        )?;

        let catalog = McpCatalog::scan(Some(home.path()), None)?;
        let registration = catalog
            .registrations()
            .iter()
            .find(|registration| registration.selector == "gemini:user:docs")
            .expect("Gemini docs registration");
        let disabled = catalog.update_basic_config(
            registration,
            &super::McpConfigPatch {
                enabled: Some(false),
                ..super::McpConfigPatch::default()
            },
        )?;
        assert!(!disabled.enabled);
        let content: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path)?)?;
        assert_eq!(content["mcp"]["allowed"], serde_json::json!(["docs"]));
        assert_eq!(content["mcp"]["excluded"], serde_json::json!(["docs"]));

        let enabled = catalog.update_basic_config(
            &disabled,
            &super::McpConfigPatch {
                enabled: Some(true),
                ..super::McpConfigPatch::default()
            },
        )?;
        assert!(enabled.enabled);
        let content: serde_json::Value = serde_json::from_str(&fs::read_to_string(path)?)?;
        assert_eq!(content["mcp"]["allowed"], serde_json::json!(["docs"]));
        assert_eq!(content["mcp"]["excluded"], serde_json::json!([]));
        Ok(())
    }

    #[test]
    fn basic_edit_refuses_jsonc_without_rewriting_comments() -> Result<()> {
        let home = tempdir()?;
        fs::create_dir_all(home.path().join(".config/opencode"))?;
        let path = home.path().join(".config/opencode/opencode.jsonc");
        let original = r#"{
  // This comment must never be normalized away.
  "mcp": {
    "docs": {
      "type": "local",
      "command": ["docs-server"]
    }
  }
}"#;
        fs::write(&path, original)?;

        let catalog = McpCatalog::scan(Some(home.path()), None)?;
        let registration = catalog
            .registrations()
            .iter()
            .find(|registration| registration.selector == "opencode:user:docs")
            .expect("OpenCode docs registration");
        let error = catalog
            .update_basic_config(
                registration,
                &super::McpConfigPatch {
                    command: Some(Some("new-server".to_owned())),
                    ..super::McpConfigPatch::default()
                },
            )
            .expect_err("JSONC must require the external editor");

        assert!(error.to_string().contains("press `o`"));
        assert_eq!(fs::read_to_string(path)?, original);
        Ok(())
    }

    #[test]
    fn basic_edit_never_writes_redaction_placeholders_over_secrets() -> Result<()> {
        let home = tempdir()?;
        fs::create_dir_all(home.path().join(".codex"))?;
        let path = home.path().join(".codex/config.toml");
        let original = r#"[mcp_servers.secure]
command = "secure-server"
args = ["--api-key", "literal-secret"]
"#;
        fs::write(&path, original)?;

        let catalog = McpCatalog::scan(Some(home.path()), None)?;
        let registration = catalog
            .registrations()
            .iter()
            .find(|registration| registration.name == "secure")
            .expect("secure registration");
        assert!(
            registration
                .args
                .iter()
                .any(|argument| argument == "<redacted>")
        );
        let error = catalog
            .update_basic_config(
                registration,
                &super::McpConfigPatch {
                    args: Some(registration.args.clone()),
                    ..super::McpConfigPatch::default()
                },
            )
            .expect_err("redacted values must never be persisted");

        assert!(error.to_string().contains("still contains `<redacted>`"));
        assert_eq!(fs::read_to_string(path)?, original);
        Ok(())
    }

    #[test]
    fn codex_stdio_registration_is_normalized_without_exposing_secrets() -> Result<()> {
        let home = tempdir()?;
        fs::create_dir_all(home.path().join(".codex"))?;
        fs::write(
            home.path().join(".codex/config.toml"),
            r#"
[mcp_servers.docs]
command = "npx"
args = ["-y", "@example/docs-mcp", "--api-key", "super-secret", "--header", "Authorization: Bearer header-secret", "https://alice:password@example.test/mcp?token=url-secret"]
env = { API_TOKEN = "literal-secret" }
env_vars = ["FORWARDED_TOKEN", { name = "LOCAL_EXTRA", source = "local" }]
startup_timeout_sec = 20.5
tool_timeout_sec = 45
required = true
enabled_tools = ["search", "read"]
disabled_tools = ["write"]
default_tools_approval_mode = "prompt"

[mcp_servers.docs.tools.search]
approval_mode = "approve"
"#,
        )?;

        let catalog = McpCatalog::scan(Some(home.path()), None)?;
        let registrations = catalog.registrations();
        assert_eq!(registrations.len(), 1);
        let server = &registrations[0];
        assert_eq!(server.selector, "codex:user:docs");
        assert_eq!(server.transport, McpTransport::Stdio);
        assert_eq!(server.command.as_deref(), Some("npx"));
        assert_eq!(
            server.args,
            [
                "-y",
                "@example/docs-mcp",
                "--api-key",
                "<redacted>",
                "--header",
                "<redacted>",
                "https://example.test/mcp?token=<redacted>",
            ]
        );
        assert_eq!(server.timeouts.startup_ms, Some(20_500));
        assert_eq!(server.timeouts.tool_ms, Some(45_000));
        assert_eq!(server.tool_policy.include, ["read", "search"]);
        assert_eq!(server.tool_policy.exclude, ["write"]);
        assert_eq!(
            server.tool_policy.approval_overrides.get("search"),
            Some(&"approve".to_owned())
        );

        let output = serde_json::to_string(server)?;
        assert!(!output.contains("super-secret"));
        assert!(!output.contains("header-secret"));
        assert!(!output.contains("url-secret"));
        assert!(!output.contains("literal-secret"));
        assert!(output.contains("API_TOKEN"));
        assert!(output.contains("FORWARDED_TOKEN"));
        assert!(output.contains("LOCAL_EXTRA"));
        let debug = format!("{catalog:?}");
        for secret in [
            "super-secret",
            "header-secret",
            "url-secret",
            "literal-secret",
        ] {
            assert!(!debug.contains(secret));
        }
        Ok(())
    }

    #[test]
    fn codex_http_registration_records_auth_and_header_sources_safely() -> Result<()> {
        let home = tempdir()?;
        let workspace = tempdir()?;
        fs::create_dir_all(workspace.path().join(".codex"))?;
        fs::write(
            workspace.path().join(".codex/config.toml"),
            r#"
[mcp_servers.remote]
url = "https://alice:password@example.com/mcp?token=secret&region=cn#private"
enabled = false
bearer_token_env_var = "REMOTE_MCP_TOKEN"
http_headers = { "X-Tenant" = "tenant-secret" }
env_http_headers = { "X-Api-Key" = "REMOTE_API_KEY" }
startup_timeout_ms = 1250
scopes = ["catalog.read", "catalog.write"]
oauth_resource = "https://alice:password@example.com/audience?tenant=secret"
"#,
        )?;

        let catalog = McpCatalog::scan(Some(home.path()), Some(workspace.path()))?;
        let server = catalog.registrations().first().expect("one server");
        assert_eq!(server.selector, "codex:project:remote");
        assert_eq!(server.transport, McpTransport::StreamableHttp);
        assert!(!server.enabled);
        assert_eq!(server.timeouts.startup_ms, Some(1_250));
        assert_eq!(
            server.url.as_deref(),
            Some("https://example.com/mcp?token=<redacted>&region=<redacted>")
        );
        assert_eq!(
            server
                .authentication
                .iter()
                .map(|auth| auth.kind.as_str())
                .collect::<Vec<_>>(),
            ["bearer_env", "oauth"]
        );
        assert_eq!(
            server.options.get("oauth_scopes"),
            Some(&serde_json::json!(["catalog.read", "catalog.write"]))
        );
        assert_eq!(
            server.options.get("oauth_resource"),
            Some(&serde_json::json!(
                "https://example.com/audience?tenant=<redacted>"
            ))
        );
        assert_eq!(
            server
                .headers
                .iter()
                .map(|header| (header.name.as_str(), header.source))
                .collect::<Vec<_>>(),
            [
                ("X-Api-Key", super::McpValueSource::Environment),
                ("X-Tenant", super::McpValueSource::Literal),
            ]
        );
        let output = serde_json::to_string(server)?;
        for secret in ["password", "secret", "tenant-secret"] {
            assert!(!output.contains(secret));
        }
        Ok(())
    }

    #[test]
    fn catalog_filters_and_requires_disambiguation_for_duplicate_names() -> Result<()> {
        let home = tempdir()?;
        let workspace = tempdir()?;
        fs::create_dir_all(home.path().join(".codex"))?;
        fs::create_dir_all(workspace.path().join(".codex"))?;
        fs::write(
            home.path().join(".codex/config.toml"),
            "[mcp_servers.docs]\ncommand = \"user-docs\"\n",
        )?;
        fs::write(
            workspace.path().join(".codex/config.toml"),
            "[mcp_servers.docs]\ncommand = \"project-docs\"\n",
        )?;

        let catalog = McpCatalog::scan(Some(home.path()), Some(workspace.path()))?;
        let error = catalog
            .inspect("docs", None, None, None)
            .expect_err("a bare duplicate name must be rejected");
        let message = error.to_string();
        assert!(message.contains("ambiguous MCP registration `docs`"));
        assert!(message.contains("codex:user:docs"));
        assert!(message.contains("codex:project:docs"));

        let detail = catalog.inspect("docs", Some("codex"), Some("project"), None)?;
        assert_eq!(detail.registration.command.as_deref(), Some("project-docs"));
        assert!(detail.probe.is_none());

        let selected = catalog.select(Some("codex"), None, Some(".codex/config.toml"))?;
        assert_eq!(selected.len(), 2);
        assert!(
            catalog
                .select(Some("unknown"), None, None)
                .expect_err("unknown providers must be actionable")
                .to_string()
                .contains("unsupported MCP provider")
        );
        Ok(())
    }

    #[test]
    fn discovers_json_client_registrations_across_user_local_and_project_scopes() -> Result<()> {
        let home = tempdir()?;
        let workspace = home.path().join("work/project");
        fs::create_dir_all(&workspace)?;
        fs::create_dir_all(home.path().join(".cursor"))?;
        fs::create_dir_all(home.path().join(".gemini"))?;
        fs::create_dir_all(home.path().join(".config/opencode"))?;

        let workspace_key = workspace.to_string_lossy().replace('\\', "\\\\");
        fs::write(
            home.path().join(".claude.json"),
            format!(
                r#"{{
  "mcpServers": {{
    "claude-user": {{"command": "claude-user", "env": {{"TOKEN": "hidden"}}}}
  }},
  "projects": {{
    "{workspace_key}": {{
      "mcpServers": {{"claude-local": {{"type": "http", "url": "https://example.test/mcp"}}}}
    }}
  }}
}}"#
            ),
        )?;
        fs::write(
            workspace.join(".mcp.json"),
            r#"{"mcpServers":{"claude-project":{"command":"project-server"}}}"#,
        )?;
        fs::write(
            home.path().join(".cursor/mcp.json"),
            r#"{"mcpServers":{"cursor-docs":{"url":"https://cursor.test/mcp","headers":{"Authorization":"secret"}}}}"#,
        )?;
        fs::write(
            home.path().join(".gemini/settings.json"),
            r#"{"mcpServers":{"gemini-search":{"httpUrl":"https://gemini.test/mcp","includeTools":["search"],"excludeTools":["delete"],"timeout":20000,"trust":true,"futureFlag":"present"}}}"#,
        )?;
        fs::write(
            home.path().join(".config/opencode/opencode.jsonc"),
            r#"{
              // OpenCode v1 configuration
              "mcp": {
                "opencode-local": {
                  "type": "local",
                  "command": ["npx", "-y", "server"],
                  "environment": {"API_KEY": "opencode-secret"},
                  "enabled": true
                }
              }
            }"#,
        )?;

        let catalog = McpCatalog::scan(Some(home.path()), Some(&workspace))?;
        let selectors: Vec<&str> = catalog
            .registrations()
            .iter()
            .map(|registration| registration.selector.as_str())
            .collect();
        assert_eq!(
            selectors,
            [
                "claude:local:claude-local",
                "claude:project:claude-project",
                "claude:user:claude-user",
                "cursor:user:cursor-docs",
                "gemini:user:gemini-search",
                "opencode:user:opencode-local",
            ]
        );
        let gemini = catalog
            .registrations()
            .iter()
            .find(|registration| registration.name == "gemini-search")
            .expect("Gemini registration");
        assert_eq!(gemini.transport, McpTransport::StreamableHttp);
        assert_eq!(gemini.timeouts.catalog_ms, Some(20_000));
        assert_eq!(gemini.tool_policy.include, ["search"]);
        assert_eq!(gemini.tool_policy.exclude, ["delete"]);
        assert_eq!(gemini.extra_fields, ["futureFlag"]);

        let output = serde_json::to_string(catalog.registrations())?;
        for secret in ["hidden", "opencode-secret", "Authorization\":\"secret"] {
            assert!(!output.contains(secret));
        }
        Ok(())
    }

    #[test]
    fn discovers_goose_omp_and_opted_in_pi_adapter_metadata() -> Result<()> {
        let home = tempdir()?;
        let workspace = home.path().join("work/project");
        fs::create_dir_all(&workspace)?;
        fs::create_dir_all(home.path().join(".config/goose"))?;
        fs::create_dir_all(home.path().join(".omp/agent"))?;
        fs::create_dir_all(workspace.join(".omp"))?;
        fs::create_dir_all(home.path().join(".pi/agent"))?;
        fs::create_dir_all(home.path().join(".config/mcp"))?;
        fs::write(
            home.path().join(".config/goose/config.yaml"),
            r"extensions:
  goose-tools:
    name: Goose Tools
    description: Local tool catalog
    enabled: true
    type: stdio
    cmd: goose-mcp
    args: [serve]
    env_keys: [GOOSE_TOKEN]
    available_tools: [lookup, summarize]
    timeout: 30
",
        )?;
        fs::write(
            home.path().join(".omp/agent/mcp.json"),
            r#"{
              "mcpServers": {
                "omp-remote": {
                  "type": "http",
                  "url": "https://omp.test/mcp",
                  "headers": {"Authorization": "!security find-generic-password"}
                },
                "omp-off": {"command": "off-server"}
              },
              "disabledServers": ["omp-off"]
            }"#,
        )?;
        fs::write(
            workspace.join(".omp/mcp.json"),
            r#"{"mcpServers":{"omp-project":{"command":"project-omp"}}}"#,
        )?;
        fs::write(
            home.path().join(".pi/agent/settings.json"),
            r#"{"packages":["github:example/pi-mcp-adapter"]}"#,
        )?;
        fs::write(
            home.path().join(".pi/agent/mcp.json"),
            r#"{"mcpServers":{"pi-user":{"command":"pi-server"}}}"#,
        )?;
        fs::write(
            home.path().join(".config/mcp/mcp.json"),
            r#"{"mcpServers":{"pi-shared":{"url":"https://pi.test/mcp"}}}"#,
        )?;

        let catalog = McpCatalog::scan(Some(home.path()), Some(&workspace))?;
        let registrations = catalog.registrations();
        for selector in [
            "goose:user:goose-tools",
            "omp:project:omp-project",
            "omp:user:omp-off",
            "omp:user:omp-remote",
            "pi:shared:pi-shared",
            "pi:user:pi-user",
        ] {
            assert!(
                registrations
                    .iter()
                    .any(|registration| registration.selector == selector),
                "missing {selector}"
            );
        }
        let goose = registrations
            .iter()
            .find(|registration| registration.name == "goose-tools")
            .expect("Goose extension");
        assert_eq!(goose.display_name.as_deref(), Some("Goose Tools"));
        assert_eq!(goose.tool_policy.include, ["lookup", "summarize"]);
        assert_eq!(goose.timeouts.catalog_ms, Some(30_000));
        assert_eq!(goose.environment[0].name, "GOOSE_TOKEN");
        assert_eq!(goose.environment[0].source, McpValueSource::Forwarded);

        let omp_off = registrations
            .iter()
            .find(|registration| registration.name == "omp-off")
            .expect("disabled OMP server");
        assert!(!omp_off.enabled);
        let omp_remote = registrations
            .iter()
            .find(|registration| registration.name == "omp-remote")
            .expect("remote OMP server");
        assert_eq!(omp_remote.headers[0].source, McpValueSource::DynamicCommand);
        assert!(
            omp_remote
                .warnings
                .iter()
                .any(|warning| warning.contains("never executed"))
        );
        assert!(!serde_json::to_string(registrations)?.contains("find-generic-password"));
        Ok(())
    }

    #[test]
    fn explicit_probe_refuses_disabled_registration_without_starting_it() -> Result<()> {
        let home = tempdir()?;
        fs::create_dir_all(home.path().join(".codex"))?;
        let sentinel = home.path().join("must-not-exist");
        fs::write(
            home.path().join(".codex/config.toml"),
            format!(
                "[mcp_servers.disabled]\ncommand = \"sh\"\nargs = [\"-c\", \"touch {}\"]\nenabled = false\n",
                sentinel.display()
            ),
        )?;

        let catalog = McpCatalog::scan(Some(home.path()), None)?;
        let detail = catalog.inspect_with_probe("disabled", None, None, None, 1)?;
        let probe = detail.probe.expect("probe decision");
        assert_eq!(probe.status, super::McpProbeStatus::Refused);
        assert!(
            probe
                .error
                .as_deref()
                .is_some_and(|error| error.contains("disabled"))
        );
        assert!(
            !sentinel.exists(),
            "disabled command was unexpectedly started"
        );
        Ok(())
    }

    #[test]
    fn explicit_probe_refuses_dynamic_value_commands_without_executing_them() -> Result<()> {
        let home = tempdir()?;
        fs::create_dir_all(home.path().join(".omp/agent"))?;
        let sentinel = home.path().join("dynamic-command-must-not-run");
        fs::write(
            home.path().join(".omp/agent/mcp.json"),
            format!(
                r#"{{"mcpServers":{{"dynamic":{{
                    "type":"http",
                    "url":"http://127.0.0.1:9/mcp",
                    "headers":{{"Authorization":"!touch {}"}}
                }}}}}}"#,
                sentinel.display()
            ),
        )?;

        let catalog = McpCatalog::scan(Some(home.path()), None)?;
        let detail = catalog.inspect_with_probe("dynamic", Some("omp"), None, None, 1)?;
        let probe = detail.probe.expect("probe decision");
        assert_eq!(probe.status, super::McpProbeStatus::Failed);
        assert!(
            probe
                .error
                .as_deref()
                .is_some_and(|error| error.contains("dynamic value"))
        );
        assert!(!sentinel.exists(), "dynamic value command was executed");
        Ok(())
    }

    #[test]
    fn explicit_probe_does_not_run_remote_codex_stdio_locally() -> Result<()> {
        let home = tempdir()?;
        fs::create_dir_all(home.path().join(".codex"))?;
        let sentinel = home.path().join("remote-command-must-not-run-locally");
        fs::write(
            home.path().join(".codex/config.toml"),
            format!(
                "[mcp_servers.remote]\ncommand = \"sh\"\nargs = [\"-c\", \"touch {}\"]\nexperimental_environment = \"remote\"\nenv_vars = [{{ name = \"REMOTE_TOKEN\", source = \"remote\" }}]\n",
                sentinel.display()
            ),
        )?;

        let catalog = McpCatalog::scan(Some(home.path()), None)?;
        let registration = &catalog.registrations()[0];
        assert_eq!(
            registration.options.get("execution_environment"),
            Some(&serde_json::json!("remote"))
        );
        assert!(registration.environment.iter().any(|binding| {
            binding.name == "REMOTE_TOKEN"
                && binding.source == super::McpValueSource::RemoteEnvironment
        }));
        let detail = catalog.inspect_with_probe("remote", Some("codex"), None, None, 1)?;
        let probe = detail.probe.expect("probe decision");
        assert_eq!(probe.status, super::McpProbeStatus::Unsupported);
        assert!(
            probe
                .error
                .as_deref()
                .is_some_and(|error| error.contains("remote executor"))
        );
        assert!(!sentinel.exists(), "remote command was started locally");
        Ok(())
    }

    #[test]
    fn discovers_only_enabled_claude_and_codex_plugin_mcp_servers() -> Result<()> {
        let home = tempdir()?;
        let workspace = home.path().join("work/project");
        let claude_plugin = home.path().join(".claude/plugins/cache/acme/db/1.0.0");
        let codex_marketplace = home.path().join("codex-marketplace");
        let codex_plugin = codex_marketplace.join("plugins/computer-use");
        fs::create_dir_all(claude_plugin.join(".claude-plugin"))?;
        fs::create_dir_all(codex_plugin.join(".codex-plugin"))?;
        fs::create_dir_all(home.path().join(".claude/plugins"))?;
        fs::create_dir_all(home.path().join(".codex"))?;
        fs::create_dir_all(&workspace)?;
        fs::write(
            home.path().join(".claude/settings.json"),
            r#"{"enabledPlugins":{"db@acme":true,"off@acme":false}}"#,
        )?;
        fs::write(
            home.path().join(".claude/plugins/installed_plugins.json"),
            format!(
                r#"{{"version":2,"plugins":{{"db@acme":[{{"scope":"user","installPath":{},"version":"1.0.0"}}]}}}}"#,
                serde_json::to_string(&claude_plugin)?
            ),
        )?;
        fs::write(
            claude_plugin.join(".claude-plugin/plugin.json"),
            r#"{"name":"db","mcpServers":"./mcp.json"}"#,
        )?;
        fs::write(
            claude_plugin.join("mcp.json"),
            r#"{"mcpServers":{"plugin-db":{"type":"http","url":"https://plugin.test/mcp","headers":{"Authorization":"Bearer ${PLUGIN_TOKEN}"}}}}"#,
        )?;
        fs::write(
            codex_plugin.join(".codex-plugin/plugin.json"),
            r#"{"name":"computer-use"}"#,
        )?;
        fs::write(
            codex_plugin.join(".mcp.json"),
            r#"{"computer-use":{"command":"./bin/server","args":["mcp"],"cwd":".","env_vars":["DISPLAY"]}}"#,
        )?;
        fs::write(
            home.path().join(".codex/config.toml"),
            format!(
                r#"[marketplaces.bundle]
source_type = "local"
source = {}

[plugins."computer-use@bundle"]
enabled = true

[plugins."off@bundle"]
enabled = false
"#,
                toml::Value::String(codex_marketplace.to_string_lossy().into_owned())
            ),
        )?;

        let catalog = McpCatalog::scan(Some(home.path()), Some(&workspace))?;
        let plugins: Vec<_> = catalog
            .registrations()
            .iter()
            .filter(|registration| registration.scope == "plugin")
            .collect();
        assert_eq!(plugins.len(), 2);
        assert_eq!(plugins[0].selector, "claude:plugin:plugin-db");
        assert_eq!(
            plugins[0].options.get("plugin_id"),
            Some(&serde_json::json!("db@acme"))
        );
        assert_eq!(plugins[0].headers[0].source, McpValueSource::Environment);
        assert_eq!(plugins[1].selector, "codex:plugin:computer-use");
        let codex_plugin_canonical = codex_plugin.canonicalize()?;
        assert_eq!(
            plugins[1].cwd.as_deref(),
            Some(codex_plugin_canonical.as_path())
        );
        assert_eq!(plugins[1].environment[0].name, "DISPLAY");
        assert_eq!(plugins[1].environment[0].source, McpValueSource::Forwarded);
        Ok(())
    }
}
