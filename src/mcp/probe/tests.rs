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
        let mut tool =
            Tool::new("search", "Search documentation", schema).with_title("Documentation search");
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
