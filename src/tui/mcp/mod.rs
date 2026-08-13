pub(crate) mod app;
pub(crate) mod event;
pub(crate) mod render;

use anyhow::Result;

use crate::mcp::{McpDetail, McpRegistration};

pub(crate) fn manage_mcp(
    registrations: Vec<McpRegistration>,
    probe: impl FnMut(&McpRegistration) -> Result<McpDetail> + Send + 'static,
    locate: impl FnMut(&McpRegistration) -> Result<usize>,
    refresh: impl FnMut() -> Result<Vec<McpRegistration>>,
    delete: impl FnMut(&McpRegistration) -> Result<()>,
) -> Result<()> {
    event::run_mcp_browser(registrations, probe, locate, refresh, delete)
}
