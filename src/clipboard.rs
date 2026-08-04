use anyhow::{Context, Result};

use crate::export::render_session_detail_markdown;
use crate::session::{DetailScope, SessionDetail};

/// Lazily initialized system clipboard retained for the lifetime of the TUI.
#[derive(Default)]
pub struct SessionClipboard {
    clipboard: Option<arboard::Clipboard>,
}

impl SessionClipboard {
    pub fn copy_detail(&mut self, detail: &SessionDetail, scope: DetailScope) -> Result<()> {
        let clipboard = match self.clipboard.as_mut() {
            Some(clipboard) => clipboard,
            None => self.clipboard.insert(
                arboard::Clipboard::new().context("failed to access the system clipboard")?,
            ),
        };
        clipboard
            .set_text(render_session_detail_markdown(detail, scope))
            .context("failed to copy the session detail to the system clipboard")
    }
}
