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
    read: impl FnMut(&Path) -> Result<String>,
    delete: impl FnMut(&Path) -> Result<PathBuf>,
) -> Result<Option<Surface>> {
    event::run_memory_browser(files, read, delete)
}
