pub(crate) mod app;
pub(crate) mod event;
pub(crate) mod render;

#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::memory::MemoryFile;
use crate::tui::common::Surface;

pub(crate) fn manage_memories(
    files: Vec<MemoryFile>,
    saved: &mut Option<app::MemoriesApp>,
    read: impl FnMut(&Path) -> Result<String>,
    delete: impl FnMut(&Path) -> Result<PathBuf>,
) -> Result<Option<Surface>> {
    let app = saved.get_or_insert_with(|| app::MemoriesApp::new(files.clone()));
    if app.files != files {
        let selected = app.selected_file().map(|file| file.path.clone());
        app.files = files;
        app.recompute_filter();
        if let Some(index) = selected.and_then(|path| {
            app.visible
                .iter()
                .position(|index| app.files[*index].path == path)
        }) {
            app.selected_index = index;
        }
    }
    event::run_memory_browser(app, read, delete)
}
