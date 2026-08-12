pub(crate) mod app;
pub(crate) mod event;
pub(crate) mod render;

use anyhow::Result;

use crate::mcp::{McpDetail, McpRegistration};

pub(crate) fn manage_mcp(
    registrations: Vec<McpRegistration>,
    probe: impl FnMut(&McpRegistration) -> Result<McpDetail> + Send + 'static,
) -> Result<()> {
    event::run_mcp_browser(registrations, probe)
}
