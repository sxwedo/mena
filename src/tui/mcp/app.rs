use std::collections::HashMap;

use anyhow::Result;
use ratatui::text::Line;

use crate::mcp::{McpDetail, McpRegistration};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum McpFocus {
    List,
    Detail,
}

pub(crate) struct McpDetailLayout {
    pub(crate) registration_index: usize,
    pub(crate) width: u16,
    pub(crate) lines: Vec<Line<'static>>,
}

pub(crate) struct McpNotice {
    pub(crate) message: String,
    pub(crate) error: bool,
}

#[allow(clippy::struct_excessive_bools)] // UI state flags; grouping them hurts readability
pub(crate) struct McpApp {
    pub(crate) registrations: Vec<McpRegistration>,
    pub(crate) visible: Vec<usize>,
    pub(crate) selected_index: usize,
    pub(crate) query: String,
    pub(crate) is_searching: bool,
    pub(crate) focus: McpFocus,
    pub(crate) detail_scroll: usize,
    pub(crate) detail_max_scroll: usize,
    pub(crate) detail_layout: Option<McpDetailLayout>,
    pub(crate) full_screen_detail: bool,
    pub(crate) marquee_offset: usize,
    pub(crate) probe_in_progress: Option<usize>,
    pub(crate) exit_after_probe: bool,
    /// Registration awaiting the lowercase-`y` probe confirmation.
    pub(crate) pending_probe: Option<usize>,
    pub(crate) pending_delete: Option<usize>,
    /// Whether the `?` keymap overlay is visible.
    pub(crate) show_help: bool,
    pub(crate) notice: Option<McpNotice>,
    search_index: Vec<String>,
    probe_details: HashMap<usize, McpDetail>,
    probe_errors: HashMap<usize, String>,
    detail_texts: HashMap<usize, String>,
}

impl McpApp {
    pub(crate) fn new(registrations: Vec<McpRegistration>) -> Self {
        let visible = (0..registrations.len()).collect();
        let search_index = registrations.iter().map(registration_search_text).collect();
        Self {
            registrations,
            visible,
            selected_index: 0,
            query: String::new(),
            is_searching: false,
            focus: McpFocus::List,
            detail_scroll: 0,
            detail_max_scroll: 0,
            detail_layout: None,
            full_screen_detail: false,
            marquee_offset: 0,
            probe_in_progress: None,
            exit_after_probe: false,
            pending_probe: None,
            pending_delete: None,
            show_help: false,
            notice: None,
            search_index,
            probe_details: HashMap::new(),
            probe_errors: HashMap::new(),
            detail_texts: HashMap::new(),
        }
    }

    pub(crate) fn recompute_filter(&mut self) {
        let previously_selected = self.selected_catalog_index();
        let terms = self
            .query
            .split_ascii_whitespace()
            .map(str::to_ascii_lowercase)
            .collect::<Vec<_>>();
        self.visible = self
            .search_index
            .iter()
            .enumerate()
            .filter(|(_, text)| terms.iter().all(|term| text.contains(term)))
            .map(|(index, _)| index)
            .collect();
        self.selected_index = previously_selected
            .and_then(|selected| self.visible.iter().position(|index| *index == selected))
            .unwrap_or_default();
        if self.selected_catalog_index() != previously_selected {
            self.reset_detail_scroll();
        }
    }

    pub(crate) fn selected_catalog_index(&self) -> Option<usize> {
        self.visible.get(self.selected_index).copied()
    }

    pub(crate) fn selected_registration(&self) -> Option<&McpRegistration> {
        self.selected_catalog_index()
            .and_then(|index| self.registrations.get(index))
    }

    pub(crate) fn selected_probe_error(&self) -> Option<&str> {
        self.selected_catalog_index()
            .and_then(|index| self.probe_errors.get(&index))
            .map(String::as_str)
    }

    pub(crate) fn selected_detail_text(&mut self) -> Option<&str> {
        let index = self.selected_catalog_index()?;
        if !self.detail_texts.contains_key(&index) {
            let probe = self
                .probe_details
                .get(&index)
                .and_then(|detail| detail.probe.as_ref());
            let text =
                crate::view::render_mcp_registration_detail(&self.registrations[index], probe);
            self.detail_texts.insert(index, text);
        }
        self.detail_texts.get(&index).map(String::as_str)
    }

    pub(crate) const fn select_next(&mut self) {
        if self.selected_index + 1 < self.visible.len() {
            self.selected_index += 1;
            self.reset_detail_scroll();
        }
    }

    pub(crate) const fn select_previous(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
            self.reset_detail_scroll();
        }
    }

    pub(crate) const fn select_first(&mut self) {
        if !self.visible.is_empty() && self.selected_index != 0 {
            self.selected_index = 0;
            self.reset_detail_scroll();
        }
    }

    pub(crate) const fn select_last(&mut self) {
        let last = self.visible.len().saturating_sub(1);
        if !self.visible.is_empty() && self.selected_index != last {
            self.selected_index = last;
            self.reset_detail_scroll();
        }
    }

    pub(crate) fn scroll_detail(&mut self, amount: isize) {
        self.detail_scroll = self
            .detail_scroll
            .saturating_add_signed(amount)
            .min(self.detail_max_scroll);
    }

    /// Arm the probe confirmation for the selected registration. `p` always
    /// shows what will be started before anything runs.
    pub(crate) fn arm_probe(&mut self) {
        if self.probe_in_progress.is_some() || self.selected_catalog_index().is_none() {
            return;
        }
        self.pending_probe = self.selected_catalog_index();
        self.notice = None;
    }

    pub(crate) const fn cancel_probe(&mut self) {
        self.pending_probe = None;
    }

    pub(crate) fn begin_probe(&mut self) -> Option<usize> {
        if self.probe_in_progress.is_some() {
            return None;
        }
        let index = self.selected_catalog_index()?;
        self.probe_in_progress = Some(index);
        self.notice = None;
        self.exit_after_probe = false;
        self.probe_errors.remove(&index);
        self.detail_layout = None;
        Some(index)
    }

    pub(crate) fn begin_external_edit(&mut self) -> Option<usize> {
        if self.probe_in_progress.is_some() {
            self.notice = Some(McpNotice {
                message: "wait for the active probe before editing configuration".to_owned(),
                error: true,
            });
            return None;
        }
        let index = self.selected_catalog_index()?;
        match crate::mcp::ensure_source_editable(&self.registrations[index]) {
            Ok(()) => {
                self.notice = None;
                Some(index)
            }
            Err(error) => {
                self.notice = Some(McpNotice {
                    message: format!("{error:#}"),
                    error: true,
                });
                None
            }
        }
    }

    pub(crate) fn begin_delete(&mut self) {
        if self.probe_in_progress.is_some() {
            self.notice = Some(McpNotice {
                message: "wait for the active probe before deleting configuration".to_owned(),
                error: true,
            });
            return;
        }
        let Some(index) = self.selected_catalog_index() else {
            return;
        };
        match crate::mcp::ensure_config_deletable(&self.registrations[index]) {
            Ok(()) => {
                self.pending_delete = Some(index);
                self.notice = None;
            }
            Err(error) => {
                self.notice = Some(McpNotice {
                    message: format!("{error:#}"),
                    error: true,
                });
            }
        }
    }

    pub(crate) const fn cancel_delete(&mut self) {
        self.pending_delete = None;
    }

    pub(crate) fn finish_source_action(
        &mut self,
        index: usize,
        action: &str,
        completed: &str,
        result: Result<Vec<McpRegistration>>,
    ) {
        let source = self.registrations.get(index).map_or_else(
            || "selected MCP source".to_owned(),
            |registration| registration.source.display().to_string(),
        );
        self.notice = Some(match result {
            Ok(registrations) => {
                self.replace_registrations(registrations, index);
                McpNotice {
                    message: format!("{completed} {source}"),
                    error: false,
                }
            }
            Err(error) => McpNotice {
                message: format!("could not {action} config: {error:#}"),
                error: true,
            },
        });
    }

    pub(crate) fn finish_delete(&mut self, index: usize, result: Result<Vec<McpRegistration>>) {
        let selector = self.registrations.get(index).map_or_else(
            || "selected MCP registration".to_owned(),
            |registration| registration.selector.clone(),
        );
        self.pending_delete = None;
        self.notice = Some(match result {
            Ok(registrations) => {
                self.replace_registrations(registrations, index);
                McpNotice {
                    message: format!("deleted {selector}"),
                    error: false,
                }
            }
            Err(error) => McpNotice {
                message: format!("{error:#}"),
                error: true,
            },
        });
    }

    fn replace_registrations(&mut self, registrations: Vec<McpRegistration>, old_index: usize) {
        let selected_identity = self
            .registrations
            .get(old_index)
            .map(|registration| (registration.selector.clone(), registration.source.clone()));
        let old_visible_position = self.selected_index;
        self.registrations = registrations;
        self.search_index = self
            .registrations
            .iter()
            .map(registration_search_text)
            .collect();
        self.probe_details.clear();
        self.probe_errors.clear();
        self.detail_texts.clear();
        self.detail_layout = None;
        self.visible.clear();
        self.selected_index = 0;
        self.recompute_filter();
        self.selected_index = selected_identity
            .and_then(|(selector, source)| {
                self.visible.iter().position(|index| {
                    self.registrations[*index].selector == selector
                        && self.registrations[*index].source == source
                })
            })
            .unwrap_or_else(|| old_visible_position.min(self.visible.len().saturating_sub(1)));
        self.reset_detail_scroll();
    }

    pub(crate) fn finish_probe(&mut self, index: usize, result: Result<McpDetail>) {
        if self.probe_in_progress == Some(index) {
            self.probe_in_progress = None;
        }
        let result = result.and_then(|detail| {
            let expected = self.registrations.get(index).ok_or_else(|| {
                anyhow::anyhow!("MCP probe returned an out-of-range catalog index")
            })?;
            if detail.registration != *expected {
                anyhow::bail!(
                    "MCP probe result for `{}` did not match selected registration `{}`",
                    detail.registration.selector,
                    expected.selector
                );
            }
            Ok(detail)
        });
        match result {
            Ok(detail) => {
                self.probe_details.insert(index, detail);
                self.probe_errors.remove(&index);
            }
            Err(error) => {
                self.probe_errors.insert(index, format!("{error:#}"));
            }
        }
        self.detail_texts.remove(&index);
        self.detail_layout = None;
        if self.selected_catalog_index() == Some(index) {
            self.reset_detail_scroll();
        }
    }

    const fn reset_detail_scroll(&mut self) {
        self.detail_scroll = 0;
        self.detail_max_scroll = 0;
    }
}

fn registration_search_text(registration: &McpRegistration) -> String {
    let mut fields = vec![
        registration.selector.as_str(),
        registration.name.as_str(),
        registration.provider.as_str(),
        registration.scope.as_str(),
        registration.transport.as_str(),
        registration.display_name.as_deref().unwrap_or_default(),
        registration.description.as_deref().unwrap_or_default(),
        registration.command.as_deref().unwrap_or_default(),
        registration.url.as_deref().unwrap_or_default(),
    ];
    let source = registration.source.to_string_lossy();
    fields.push(source.as_ref());

    let mut text = fields.join("\n");
    for value in registration
        .args
        .iter()
        .chain(&registration.tool_policy.include)
        .chain(&registration.tool_policy.exclude)
        .chain(&registration.warnings)
    {
        text.push('\n');
        text.push_str(value);
    }
    text.make_ascii_lowercase();
    text
}

#[cfg(test)]
mod tests;
