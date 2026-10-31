pub(crate) mod app;
pub(crate) mod event;
pub(crate) mod render;

use anyhow::Result;

use crate::mcp::{McpDetail, McpRegistration};
use crate::tui::common::Surface;

pub(crate) fn manage_mcp(
    registrations: Vec<McpRegistration>,
    saved: &mut Option<app::McpApp>,
    probe: impl FnMut(&McpRegistration) -> Result<McpDetail> + Send + 'static,
    locate: impl FnMut(&McpRegistration) -> Result<usize>,
    refresh: impl FnMut() -> Result<Vec<McpRegistration>>,
    delete: impl FnMut(&McpRegistration) -> Result<()>,
) -> Result<Option<Surface>> {
    let app = saved.get_or_insert_with(|| app::McpApp::new(registrations.clone()));
    if app.registrations != registrations {
        let selected = app.selected_catalog_index().unwrap_or_default();
        app.replace_registrations(registrations, selected);
    }
    event::run_mcp_browser(app, probe, locate, refresh, delete)
}
