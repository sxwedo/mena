use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use ratatui::layout::Constraint;
use ratatui::style::Color;
use ratatui::style::Style;
use ratatui::text::{Line, Text};
use ratatui::widgets::{Paragraph, TableState, Wrap};

use super::event::session_matches;
use super::render::{fragment_detail_line, session_detail_content, session_project_label};
use crate::session::{AgentSession, DeletionSummary, DetailScope, SessionDetail};
use crate::tui::common::SessionDetailTheme;

pub(crate) type DeleteCallback<'a> = &'a mut dyn FnMut(&AgentSession) -> Result<DeletionSummary>;
pub(crate) type DetailCallback<'a> = &'a mut dyn FnMut(&AgentSession) -> Result<SessionDetail>;
pub(crate) type ExportCallback<'a> =
    &'a mut dyn FnMut(&SessionDetail, DetailScope) -> Result<PathBuf>;
pub(crate) type CopyCallback<'a> = &'a mut dyn FnMut(&SessionDetail, DetailScope) -> Result<()>;

#[derive(Default)]
pub(crate) struct SessionBrowserCallbacks<'a> {
    pub(crate) load_detail: Option<DetailCallback<'a>>,
    pub(crate) export: Option<ExportCallback<'a>>,
    pub(crate) copy: Option<CopyCallback<'a>>,
    pub(crate) delete: Option<DeleteCallback<'a>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BrowserMode {
    Browse,
    Search,
    Detail,
    ConfirmDelete,
}

/// How the session list is grouped in the table. `Flat` is the single
/// flat list; `Project` inserts selectable header rows per project, allows
/// collapsing/expanding groups via Enter, and keeps original within-group order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Grouping {
    #[default]
    Flat,
    Project,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DisplayRow {
    GroupHeader {
        project: String,
        count: usize,
        collapsed: bool,
    },
    Session {
        session_index: usize,
    },
}

impl Grouping {
    /// Cycle to the next grouping mode (bound to a key in Browse mode).
    const fn cycle(self) -> Self {
        match self {
            Self::Flat => Self::Project,
            Self::Project => Self::Flat,
        }
    }

    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Flat => "flat",
            Self::Project => "by project",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DetailAction {
    Continue,
    Resume,
}

/// A full-text message search running incrementally so the UI can show
/// progress and stay responsive (Esc cancels) while transcripts load one by
/// one. Lives on `SessionsApp::search_in_progress`; the event loop drives one
/// `step_message_search` per tick.
#[derive(Debug)]
pub(crate) struct InProgressSearch {
    /// Lowercased committed query, captured at submit time.
    pub(crate) query: String,
    /// Session indices whose transcripts match the query, accumulated as we go.
    pub(crate) hits: BTreeSet<usize>,
    /// Number of sessions whose transcripts failed to load.
    pub(crate) errors: u32,
    /// Index (into `SessionsApp::sessions`) of the next session to scan.
    pub(crate) cursor: usize,
    /// When the search started, used to animate the spinner between ticks.
    pub(crate) started: Instant,
}

pub(crate) struct SessionsApp {
    pub(crate) sessions: Vec<AgentSession>,
    pub(crate) filtered: Vec<usize>,
    pub(crate) active_targets: BTreeSet<String>,
    pub(crate) table_state: TableState,
    pub(crate) query: String,
    pub(crate) mode: BrowserMode,
    pub(crate) detail_theme: SessionDetailTheme,
    pub(crate) detail: Option<SessionDetail>,
    pub(crate) detail_scroll: usize,
    pub(crate) detail_max_scroll: usize,
    pub(crate) detail_primary_offsets: Vec<usize>,
    pub(crate) detail_layout: Option<DetailLayoutCache>,
    pub(crate) detail_status: Option<StatusMessage>,
    pub(crate) status: Option<StatusMessage>,
    /// Session indices whose messages contain the committed query. Populated only
    /// after the user presses Enter in Search mode, so live per-keystroke filtering
    /// stays on the cheap scalar fields while full-text search runs once on submit.
    pub(crate) message_search: Option<BTreeSet<usize>>,
    /// A message search currently running incrementally. `Some` only between
    /// pressing Enter (commit) and the scan finishing (or Esc cancelling).
    pub(crate) search_in_progress: Option<InProgressSearch>,
    /// Whether the table renders flat or with per-project group headers.
    pub(crate) grouping: Grouping,
    /// Set of project keys currently collapsed in Project grouping mode.
    pub(crate) collapsed_projects: BTreeSet<String>,
    /// Which messages the detail preview shows. Defaults to `Conversation`
    /// (user/assistant only); toggled with `p`/`Shift+P` in detail mode.
    pub(crate) preview_scope: DetailScope,
}

impl SessionsApp {
    #[cfg(test)]
    pub(crate) fn new(sessions: Vec<AgentSession>, active_targets: BTreeSet<String>) -> Self {
        Self::new_with_detail_theme(sessions, active_targets, SessionDetailTheme::default())
    }

    pub(crate) fn new_with_detail_theme(
        sessions: Vec<AgentSession>,
        active_targets: BTreeSet<String>,
        detail_theme: SessionDetailTheme,
    ) -> Self {
        let mut app = Self {
            sessions,
            filtered: Vec::new(),
            active_targets,
            table_state: TableState::default(),
            query: String::new(),
            mode: BrowserMode::Browse,
            detail_theme,
            detail: None,
            detail_scroll: 0,
            detail_max_scroll: 0,
            detail_primary_offsets: Vec::new(),
            detail_layout: None,
            detail_status: None,
            status: None,
            message_search: None,
            search_in_progress: None,
            grouping: Grouping::Flat,
            collapsed_projects: BTreeSet::default(),
            preview_scope: DetailScope::Conversation,
        };
        app.recompute_filter();
        app
    }

    pub(crate) fn recompute_filter(&mut self) {
        let query = self.query.to_ascii_lowercase();
        let scalar_hits = |session: &AgentSession| session_matches(session, &query);
        let message_hits = self.message_search.as_ref();
        self.filtered = self
            .sessions
            .iter()
            .enumerate()
            .filter(|(index, session)| {
                scalar_hits(session) || message_hits.is_some_and(|hits| hits.contains(index))
            })
            .map(|(index, _)| index)
            .collect();
        self.clamp_selection();
    }

    fn clamp_selection(&mut self) {
        let len = self.display_rows().len();
        let selected = self.table_state.selected().unwrap_or_default();
        self.table_state
            .select(len.checked_sub(1).map(|last| selected.min(last)));
    }

    pub(crate) fn display_rows(&self) -> Vec<DisplayRow> {
        match self.grouping {
            Grouping::Flat => self
                .filtered
                .iter()
                .map(|&session_index| DisplayRow::Session { session_index })
                .collect(),
            Grouping::Project => {
                let mut seen: Vec<String> = Vec::new();
                for &session_index in &self.filtered {
                    let key = session_project_label(&self.sessions[session_index]);
                    if !seen.contains(&key) {
                        seen.push(key);
                    }
                }
                let mut rows = Vec::new();
                for key in seen {
                    let count = self
                        .filtered
                        .iter()
                        .filter(|&&idx| session_project_label(&self.sessions[idx]) == key)
                        .count();
                    let collapsed = self.collapsed_projects.contains(&key);
                    rows.push(DisplayRow::GroupHeader {
                        project: key.clone(),
                        count,
                        collapsed,
                    });
                    if !collapsed {
                        for &session_index in &self.filtered {
                            if session_project_label(&self.sessions[session_index]) == key {
                                rows.push(DisplayRow::Session { session_index });
                            }
                        }
                    }
                }
                rows
            }
        }
    }

    pub(crate) fn selected_row(&self) -> Option<DisplayRow> {
        let rows = self.display_rows();
        let selected = self.table_state.selected()?;
        rows.get(selected).cloned()
    }

    pub(crate) fn toggle_project_collapse(&mut self, project: &str) {
        if self.collapsed_projects.contains(project) {
            self.collapsed_projects.remove(project);
        } else {
            self.collapsed_projects.insert(project.to_owned());
        }
        self.clamp_selection();
    }

    pub(crate) fn append_search(&mut self, value: &str) {
        self.query.push_str(value);
        self.recompute_filter();
    }

    fn collapse_all_projects(&mut self) {
        self.collapsed_projects.clear();
        for &session_index in &self.filtered {
            let key = session_project_label(&self.sessions[session_index]);
            self.collapsed_projects.insert(key);
        }
    }

    /// Toggle the list grouping (flat ↔ by project).
    pub(crate) fn cycle_grouping(&mut self) {
        let current_session_index = match self.selected_row() {
            Some(DisplayRow::Session { session_index }) => Some(session_index),
            _ => None,
        };
        self.grouping = self.grouping.cycle();
        if self.grouping == Grouping::Project {
            self.collapse_all_projects();
        }
        let display_rows = self.display_rows();
        if let Some(target_index) = current_session_index {
            if let Some(new_pos) = display_rows.iter().position(|r| {
                matches!(r, DisplayRow::Session { session_index } if *session_index == target_index)
            }) {
                self.table_state.select(Some(new_pos));
            } else {
                self.clamp_selection();
            }
        } else {
            self.clamp_selection();
        }
        self.status = Some(StatusMessage::success(format!(
            "Grouped: {}",
            self.grouping.label(),
        )));
    }

    /// Commit a full-text search over session transcripts. `hits` are the indices
    /// of sessions whose messages contain the (already lowercased) query. Clears
    /// any prior message search so a fresh Enter re-runs against current results.
    pub(crate) fn apply_message_search(&mut self, hits: BTreeSet<usize>) {
        self.message_search = Some(hits);
        self.recompute_filter();
    }

    /// Drop any committed message search, returning to scalar-only filtering.
    pub(crate) fn clear_message_search(&mut self) {
        if self.message_search.take().is_some() {
            self.recompute_filter();
        }
    }

    pub(crate) fn selected_session(&self) -> Option<&AgentSession> {
        match self.selected_row()? {
            DisplayRow::Session { session_index } => self.sessions.get(session_index),
            DisplayRow::GroupHeader { .. } => None,
        }
    }

    pub(crate) fn previous(&mut self) {
        self.move_by(-1);
    }

    pub(crate) fn next(&mut self) {
        self.move_by(1);
    }

    pub(crate) fn move_by(&mut self, amount: isize) {
        let len = self.display_rows().len();
        if len == 0 {
            self.table_state.select(None);
            return;
        }
        let selected = self.table_state.selected().unwrap_or_default();
        let selected = selected.saturating_add_signed(amount).min(len - 1);
        self.table_state.select(Some(selected));
    }

    pub(crate) fn first(&mut self) {
        let len = self.display_rows().len();
        self.table_state.select((len > 0).then_some(0));
    }

    pub(crate) fn last(&mut self) {
        let len = self.display_rows().len();
        self.table_state.select(len.checked_sub(1));
    }

    pub(crate) fn open_detail(&mut self, detail: SessionDetail) {
        self.detail = Some(detail);
        self.detail_scroll = 0;
        self.detail_max_scroll = 0;
        self.detail_primary_offsets.clear();
        self.detail_layout = None;
        self.detail_status = None;
        // Each freshly opened session starts in the conversation-only preview;
        // the user can press Shift+P to reveal tool/system messages.
        self.preview_scope = DetailScope::Conversation;
        self.mode = BrowserMode::Detail;
    }

    /// Switch the detail preview scope. Invalidates the cached layout so the
    /// next redraw reflows for the new message set, and clamps scroll.
    pub(crate) fn set_preview_scope(&mut self, scope: DetailScope) {
        if self.preview_scope == scope {
            return;
        }
        self.preview_scope = scope;
        self.detail_scroll = 0;
        self.detail_max_scroll = 0;
        self.detail_primary_offsets.clear();
        self.detail_layout = None;
    }

    pub(crate) fn close_detail(&mut self) {
        self.detail = None;
        self.detail_scroll = 0;
        self.detail_max_scroll = 0;
        self.detail_primary_offsets.clear();
        self.detail_layout = None;
        self.detail_status = None;
        self.mode = BrowserMode::Browse;
    }

    pub(crate) fn scroll_detail(&mut self, amount: isize) {
        self.detail_scroll = self
            .detail_scroll
            .saturating_add_signed(amount)
            .min(self.detail_max_scroll);
    }

    pub(crate) fn jump_detail_primary(&mut self, forward: bool) {
        let target = if forward {
            self.detail_primary_offsets
                .iter()
                .copied()
                .find(|offset| *offset > self.detail_scroll)
        } else {
            self.detail_primary_offsets
                .iter()
                .copied()
                .rev()
                .find(|offset| *offset < self.detail_scroll)
        };
        if let Some(target) = target {
            self.detail_scroll = target;
        }
    }

    pub(crate) fn request_delete(&mut self) {
        let Some(session) = self.selected_session() else {
            return;
        };
        if self.active_targets.contains(&session.target()) {
            self.status = Some(StatusMessage::error(
                "Cannot delete a session that may be attached to a running agent".to_owned(),
            ));
        } else {
            self.mode = BrowserMode::ConfirmDelete;
        }
    }

    pub(crate) fn deleted(&mut self, deleted: &AgentSession, summary: DeletionSummary) {
        let target = deleted.target();
        self.sessions
            .retain(|session| session.kind != deleted.kind || session.id != deleted.id);
        self.status = Some(StatusMessage::success(format!(
            "Permanently deleted {target}: {} files, {} directories, {} index records",
            summary.files, summary.directories, summary.index_records
        )));
        self.mode = BrowserMode::Browse;
        self.recompute_filter();
    }
}

pub(crate) struct StatusMessage {
    pub(crate) text: String,
    pub(crate) style: Style,
    pub(crate) is_error: bool,
}

impl StatusMessage {
    pub(crate) fn success(text: String) -> Self {
        Self {
            text,
            style: Style::default().fg(Color::Green),
            is_error: false,
        }
    }

    pub(crate) fn error(text: String) -> Self {
        Self {
            text,
            style: Style::default().fg(Color::Red),
            is_error: true,
        }
    }
}

pub(crate) struct SessionDetailContent {
    pub(crate) lines: Vec<Line<'static>>,
    pub(crate) primary_line_indices: Vec<usize>,
}

#[derive(Debug)]
pub(crate) struct DetailLayoutCache {
    pub(crate) width: u16,
    pub(crate) lines: Vec<Line<'static>>,
    pub(crate) line_offsets: Vec<usize>,
    pub(crate) primary_offsets: Vec<usize>,
}

impl DetailLayoutCache {
    pub(crate) fn new(
        detail: &SessionDetail,
        width: u16,
        theme: SessionDetailTheme,
        scope: DetailScope,
    ) -> Self {
        let width = width.max(1);
        let content = session_detail_content(detail, theme, scope);
        let mut lines = Vec::with_capacity(content.lines.len());
        let mut primary_line_indices = Vec::with_capacity(content.primary_line_indices.len());
        let mut primary_indices = content.primary_line_indices.into_iter().peekable();

        for (index, line) in content.lines.into_iter().enumerate() {
            while primary_indices
                .peek()
                .is_some_and(|primary| *primary == index)
            {
                primary_line_indices.push(lines.len());
                primary_indices.next();
            }
            lines.extend(fragment_detail_line(line));
        }

        let mut line_offsets = Vec::with_capacity(lines.len() + 1);
        line_offsets.push(0usize);
        for line in &lines {
            let height = Paragraph::new(Text::from(line.clone()))
                .wrap(Wrap { trim: false })
                .line_count(width)
                .max(1);
            let next = line_offsets
                .last()
                .copied()
                .unwrap_or_default()
                .saturating_add(height);
            line_offsets.push(next);
        }
        let primary_offsets = primary_line_indices
            .into_iter()
            .filter_map(|index| line_offsets.get(index).copied())
            .collect();

        Self {
            width,
            lines,
            line_offsets,
            primary_offsets,
        }
    }

    pub(crate) fn total_height(&self) -> usize {
        self.line_offsets.last().copied().unwrap_or_default()
    }

    pub(crate) fn visible_text(
        &self,
        scroll: usize,
        viewport_height: usize,
    ) -> (Text<'static>, u16) {
        if self.lines.is_empty() || scroll >= self.total_height() {
            return (Text::default(), 0);
        }

        let start_index = self
            .line_offsets
            .partition_point(|offset| *offset <= scroll)
            .saturating_sub(1)
            .min(self.lines.len() - 1);
        let local_scroll = scroll.saturating_sub(self.line_offsets[start_index]);
        let end_offset = scroll.saturating_add(viewport_height.max(1));
        let end_index = self
            .line_offsets
            .partition_point(|offset| *offset < end_offset)
            .max(start_index + 1)
            .min(self.lines.len());

        (
            Text::from(self.lines[start_index..end_index].to_vec()),
            u16::try_from(local_scroll).unwrap_or(u16::MAX),
        )
    }
}

pub(crate) struct Column<T> {
    pub(crate) kind: T,
    pub(crate) label: &'static str,
    pub(crate) constraint: Constraint,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionColumn {
    Target,
    Active,
    Agent,
    Project,
    Title,
    Updated,
}
