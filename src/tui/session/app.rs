use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::Result;
use ratatui::layout::Constraint;
use ratatui::style::Style;
use ratatui::text::{Line, Text};
use ratatui::widgets::{Paragraph, TableState, Wrap};

use super::event::session_matches;
use super::render::{fragment_detail_line, session_detail_content, session_project_label};
use crate::session::{AgentSession, DeletionSummary, DetailScope, SessionDetail};
use crate::tui::common::{SessionDetailTheme, UI};

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
    /// Incremental text search inside the detail popup (`/`), vim-style:
    /// `n`/`N` step through matches after Enter commits.
    DetailSearch,
    ConfirmDelete,
}

/// An in-detail text search over rendered transcript lines. `match_lines`
/// index into `DetailLayoutCache::lines`; `cursor` is the focused match.
#[derive(Debug, Clone)]
pub(crate) struct DetailSearchState {
    pub(crate) query: String,
    pub(crate) match_lines: Vec<usize>,
    pub(crate) cursor: usize,
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
    ContinueWith,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SessionBrowserResult {
    Resume(AgentSession),
    ContinueWith(AgentSession),
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
    pub(crate) protected_targets: BTreeSet<String>,
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
    /// Targets carrying a delete mark. Keyed by target (not index) so marks
    /// survive filtering, searching, regrouping, and deletions.
    pub(crate) marked_targets: BTreeSet<String>,
    /// Sessions awaiting the lowercase-`y` confirmation. Empty outside
    /// `ConfirmDelete` mode; holds either one session or every marked one.
    pub(crate) confirm_delete_targets: Vec<AgentSession>,
    /// Active text search inside the detail popup, if any. Line numbers are
    /// invalidated (and the search dropped) whenever the layout rebuilds.
    pub(crate) detail_search: Option<DetailSearchState>,
}

impl SessionsApp {
    #[cfg(test)]
    pub(crate) fn new(sessions: Vec<AgentSession>, active_targets: BTreeSet<String>) -> Self {
        Self::new_with_detail_theme(
            sessions,
            active_targets.clone(),
            active_targets,
            SessionDetailTheme::default(),
        )
    }

    pub(crate) fn new_with_detail_theme(
        sessions: Vec<AgentSession>,
        active_targets: BTreeSet<String>,
        protected_targets: BTreeSet<String>,
        detail_theme: SessionDetailTheme,
    ) -> Self {
        let mut app = Self {
            sessions,
            filtered: Vec::new(),
            active_targets,
            protected_targets,
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
            marked_targets: BTreeSet::default(),
            confirm_delete_targets: Vec::new(),
            detail_search: None,
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
        // Rebuilding the layout reflows every line, so any match positions
        // from the previous scope are meaningless; drop the search.
        self.detail_search = None;
    }

    pub(crate) fn close_detail(&mut self) {
        self.detail = None;
        self.detail_scroll = 0;
        self.detail_max_scroll = 0;
        self.detail_primary_offsets.clear();
        self.detail_layout = None;
        self.detail_status = None;
        self.detail_search = None;
        self.mode = BrowserMode::Browse;
    }

    /// Begin an incremental search over the rendered detail lines.
    pub(crate) fn begin_detail_search(&mut self) {
        self.detail_search = Some(DetailSearchState {
            query: String::new(),
            match_lines: Vec::new(),
            cursor: 0,
        });
        self.mode = BrowserMode::DetailSearch;
    }

    pub(crate) fn append_detail_search(&mut self, character: char) {
        if let Some(search) = self.detail_search.as_mut() {
            search.query.push(character);
            self.refresh_detail_matches();
        }
    }

    pub(crate) fn backspace_detail_search(&mut self) {
        if let Some(search) = self.detail_search.as_mut() {
            search.query.pop();
            self.refresh_detail_matches();
        }
    }

    /// Keep the search and return to normal detail navigation.
    pub(crate) fn commit_detail_search(&mut self) {
        if self
            .detail_search
            .as_ref()
            .is_none_or(|search| search.query.trim().is_empty())
        {
            self.detail_search = None;
        }
        if self.mode == BrowserMode::DetailSearch {
            self.mode = BrowserMode::Detail;
        }
    }

    /// Drop the search entirely and return to normal detail navigation.
    pub(crate) fn cancel_detail_search(&mut self) {
        self.detail_search = None;
        if self.mode == BrowserMode::DetailSearch {
            self.mode = BrowserMode::Detail;
        }
    }

    /// Move to the next (`n`) or previous (`N`) match, wrapping around.
    pub(crate) fn step_detail_match(&mut self, forward: bool) {
        let Some(search) = self.detail_search.as_mut() else {
            return;
        };
        if search.match_lines.is_empty() {
            return;
        }
        let len = search.match_lines.len();
        search.cursor = if forward {
            (search.cursor + 1) % len
        } else {
            (search.cursor + len - 1) % len
        };
        self.focus_current_detail_match();
    }

    fn refresh_detail_matches(&mut self) {
        let query = self
            .detail_search
            .as_ref()
            .map(|search| search.query.trim().to_ascii_lowercase())
            .unwrap_or_default();
        let match_lines = if query.is_empty() {
            Vec::new()
        } else {
            self.detail_layout
                .as_ref()
                .map(|layout| {
                    layout
                        .lines
                        .iter()
                        .enumerate()
                        .filter(|(_, line)| {
                            let mut text = String::new();
                            for span in &line.spans {
                                text.push_str(&span.content);
                            }
                            text.to_ascii_lowercase().contains(&query)
                        })
                        .map(|(index, _)| index)
                        .collect()
                })
                .unwrap_or_default()
        };
        if let Some(search) = self.detail_search.as_mut() {
            search.match_lines = match_lines;
            search.cursor = 0;
        }
        self.focus_current_detail_match();
    }

    /// Scroll the detail popup so the focused match sits at the viewport top.
    fn focus_current_detail_match(&mut self) {
        let Some(search) = self.detail_search.as_ref() else {
            return;
        };
        if let Some(&line) = search.match_lines.get(search.cursor)
            && let Some(offset) = self
                .detail_layout
                .as_ref()
                .and_then(|layout| layout.line_offsets.get(line).copied())
        {
            self.detail_scroll = offset.min(self.detail_max_scroll);
        }
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

    /// Toggle the delete mark on the selected session row. Group headers and
    /// empty selections are ignored; marks survive later filtering.
    pub(crate) fn toggle_mark(&mut self) {
        let Some(session) = self.selected_session() else {
            return;
        };
        let target = session.target();
        if !self.marked_targets.remove(&target) {
            self.marked_targets.insert(target);
        }
    }

    /// Toggle the mark on every visible session (`a`). Select-all only extends
    /// a batch Space has started: without any mark multi-select is not open
    /// yet, so nothing is marked and a status explains how to begin.
    pub(crate) fn toggle_mark_all(&mut self) {
        if self.marked_count() == 0 {
            self.status = Some(StatusMessage::error(
                "Press Space on a session to start multi-select first".to_owned(),
            ));
            return;
        }
        self.toggle_mark_visible();
    }

    /// Mark every visible session, or clear the visible marks when all of
    /// them already carry one. Marks on hidden rows are untouched.
    pub(crate) fn toggle_mark_visible(&mut self) {
        let visible: BTreeSet<String> = self
            .filtered
            .iter()
            .map(|&index| self.sessions[index].target())
            .collect();
        if visible.is_subset(&self.marked_targets) {
            self.marked_targets
                .retain(|target| !visible.contains(target));
        } else {
            self.marked_targets.extend(visible);
        }
    }

    /// Sessions in catalog order whose targets carry a delete mark.
    pub(crate) fn marked_sessions(&self) -> Vec<AgentSession> {
        self.sessions
            .iter()
            .filter(|session| self.marked_targets.contains(&session.target()))
            .cloned()
            .collect()
    }

    /// Number of sessions carrying a delete mark.
    pub(crate) fn marked_count(&self) -> usize {
        self.sessions
            .iter()
            .filter(|session| self.marked_targets.contains(&session.target()))
            .count()
    }

    pub(crate) fn request_delete(&mut self) {
        let mut targets = self.marked_sessions();
        if targets.is_empty()
            && let Some(session) = self.selected_session()
        {
            targets.push(session.clone());
        }
        if targets.is_empty() {
            return;
        }
        // Protected sessions stay untouched: a running agent whose exact
        // session cannot be pinned down protects its whole provider catalog,
        // so those targets are skipped instead of blocking the batch.
        let (deletable, skipped): (Vec<AgentSession>, Vec<AgentSession>) = targets
            .into_iter()
            .partition(|session| !self.protected_targets.contains(&session.target()));
        if deletable.is_empty() {
            self.status = Some(StatusMessage::error(format!(
                "Cannot delete {}: a running agent may own it and its exact session cannot be confirmed, so the whole provider stays protected until that agent exits",
                skipped[0].target()
            )));
            return;
        }
        if !skipped.is_empty() {
            self.status = Some(StatusMessage::success(if skipped.len() == 1 {
                "1 protected session skipped — still marked, not deletable while an agent may own it".to_owned()
            } else {
                format!(
                    "{} protected sessions skipped — still marked, not deletable while an agent may own them",
                    skipped.len()
                )
            }));
        }
        self.confirm_delete_targets = deletable;
        self.mode = BrowserMode::ConfirmDelete;
    }

    /// Remove one deleted session (and every duplicate catalog row for it)
    /// from the browsing state, clearing any mark it carried.
    pub(crate) fn apply_deletion(&mut self, deleted: &AgentSession) {
        self.marked_targets.remove(&deleted.target());
        self.sessions
            .retain(|session| session.kind != deleted.kind || session.id != deleted.id);
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
            style: Style::default().fg(UI.success),
            is_error: false,
        }
    }

    pub(crate) fn error(text: String) -> Self {
        Self {
            text,
            style: Style::default().fg(UI.danger),
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

    /// The rendered line range `[start, end)` visible at `scroll`, plus the
    /// scroll offset within the first visible line.
    pub(crate) fn visible_span(
        &self,
        scroll: usize,
        viewport_height: usize,
    ) -> (usize, usize, u16) {
        if self.lines.is_empty() || scroll >= self.total_height() {
            return (0, 0, 0);
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
            start_index,
            end_index,
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
    Marked,
    Target,
    Active,
    Agent,
    Project,
    Title,
    Updated,
}
