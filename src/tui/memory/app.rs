use std::path::PathBuf;

use crate::memory::MemoryFile;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MemoryFocus {
    List,
    Preview,
}

pub(crate) struct MemoriesApp {
    pub(crate) files: Vec<MemoryFile>,
    pub(crate) visible: Vec<usize>,
    pub(crate) selected_index: usize,
    pub(crate) query: String,
    pub(crate) is_searching: bool,
    pub(crate) focus: MemoryFocus,
    /// Cached preview for the selected file: `(path, Ok(content) | Err(message))`.
    pub(crate) preview: Option<(PathBuf, Result<String, String>)>,
    pub(crate) preview_scroll: usize,
    /// Rendered-line count clamp, recomputed while drawing.
    pub(crate) preview_max: usize,
    pub(crate) pending_delete: Option<usize>,
    pub(crate) show_help: bool,
    /// Footer status message and whether it is an error.
    pub(crate) status: Option<(String, bool)>,
}

impl MemoriesApp {
    pub(crate) fn new(files: Vec<MemoryFile>) -> Self {
        let visible = (0..files.len()).collect();
        let mut app = Self {
            files,
            visible,
            selected_index: 0,
            query: String::new(),
            is_searching: false,
            focus: MemoryFocus::List,
            preview: None,
            preview_scroll: 0,
            preview_max: 0,
            pending_delete: None,
            show_help: false,
            status: None,
        };
        app.recompute_filter();
        app
    }

    pub(crate) fn recompute_filter(&mut self) {
        let terms = self
            .query
            .split_ascii_whitespace()
            .map(str::to_ascii_lowercase)
            .collect::<Vec<_>>();
        self.visible = self
            .files
            .iter()
            .enumerate()
            .filter(|(_, file)| {
                let haystack = format!(
                    "{} {} {} {}",
                    file.name, file.provider, file.scope, file.location
                )
                .to_ascii_lowercase();
                terms.iter().all(|term| haystack.contains(term))
            })
            .map(|(index, _)| index)
            .collect();
        self.selected_index = self
            .selected_index
            .min(self.visible.len().saturating_sub(1));
        self.preview = None;
        self.preview_scroll = 0;
    }

    pub(crate) fn selected_file(&self) -> Option<&MemoryFile> {
        self.visible
            .get(self.selected_index)
            .and_then(|index| self.files.get(*index))
    }

    pub(crate) const fn select_next(&mut self) {
        if self.selected_index + 1 < self.visible.len() {
            self.selected_index += 1;
        }
    }

    pub(crate) const fn select_prev(&mut self) {
        self.selected_index = self.selected_index.saturating_sub(1);
    }

    pub(crate) const fn select_first(&mut self) {
        self.selected_index = 0;
    }

    pub(crate) const fn select_last(&mut self) {
        self.selected_index = self.visible.len().saturating_sub(1);
    }

    /// Arm the lowercase-`y` delete confirmation for the selected file.
    pub(crate) fn begin_delete(&mut self) {
        if self.selected_file().is_some() {
            self.pending_delete = Some(self.selected_index);
            self.status = None;
        }
    }

    /// Remove one deleted file from the browsing state.
    pub(crate) fn apply_deletion(&mut self, path: &std::path::Path) {
        self.files.retain(|file| file.path != path);
        self.pending_delete = None;
        self.recompute_filter();
    }
}
