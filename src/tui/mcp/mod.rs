pub(crate) mod app;
pub(crate) mod edit;
pub(crate) mod event;
pub(crate) mod render;

use anyhow::Result;

use crate::mcp::{McpConfigPatch, McpDetail, McpRegistration};

pub(crate) fn manage_mcp(
    registrations: Vec<McpRegistration>,
    probe: impl FnMut(&McpRegistration) -> Result<McpDetail> + Send + 'static,
    update: impl FnMut(&McpRegistration, &McpConfigPatch) -> Result<McpRegistration>,
) -> Result<()> {
    event::run_mcp_browser(registrations, probe, update)
}
