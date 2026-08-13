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
    pub(crate) pending_delete: Option<usize>,
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
            pending_delete: None,
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
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use super::McpApp;
    use crate::mcp::{
        McpDetail, McpProbe, McpProbeStatus, McpRegistration, McpSourceFormat, McpTimeouts,
        McpToolPolicy, McpTransport,
    };

    fn registration(name: &str, provider: &str, description: &str) -> McpRegistration {
        McpRegistration {
            selector: format!("{provider}:user:{name}"),
            name: name.to_owned(),
            provider: provider.to_owned(),
            scope: "user".to_owned(),
            source: PathBuf::from(format!("/{provider}/config.json")),
            source_format: McpSourceFormat::Json,
            transport: McpTransport::Stdio,
            enabled: true,
            valid: true,
            display_name: None,
            description: Some(description.to_owned()),
            command: Some(format!("{name}-server")),
            args: Vec::new(),
            url: None,
            cwd: None,
            timeouts: McpTimeouts::default(),
            authentication: Vec::new(),
            environment: Vec::new(),
            headers: Vec::new(),
            tool_policy: McpToolPolicy::default(),
            options: BTreeMap::new(),
            extra_fields: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn search_selects_the_matching_registration_for_the_detail_pane() {
        let mut app = McpApp::new(vec![
            registration("docs", "codex", "documentation"),
            registration("codegraph", "claude", "repository graph"),
        ]);

        app.query = "graph".to_owned();
        app.recompute_filter();

        assert_eq!(app.visible, [1]);
        assert_eq!(
            app.selected_registration().map(|entry| entry.name.as_str()),
            Some("codegraph")
        );
    }

    #[test]
    fn completed_probe_stays_bound_to_its_registration_after_selection_moves() {
        let first = registration("first", "codex", "first server");
        let second = registration("second", "claude", "second server");
        let mut app = McpApp::new(vec![first.clone(), second]);
        let index = app.begin_probe().expect("probe request");
        app.select_next();

        app.finish_probe(
            index,
            Ok(McpDetail {
                registration: first,
                probe: Some(McpProbe {
                    status: McpProbeStatus::Success,
                    duration_ms: 7,
                    protocol_version: Some("2025-11-25".to_owned()),
                    server: None,
                    capabilities: None,
                    instructions: None,
                    tools: Vec::new(),
                    prompts: Vec::new(),
                    resources: Vec::new(),
                    resource_templates: Vec::new(),
                    warnings: Vec::new(),
                    error: None,
                }),
            }),
        );

        assert_eq!(
            app.selected_registration().map(|entry| entry.name.as_str()),
            Some("second")
        );
        assert!(
            app.selected_detail_text()
                .expect("second detail")
                .contains("Runtime metadata: not probed")
        );

        app.select_previous();
        assert!(
            app.selected_detail_text()
                .expect("first detail")
                .contains("Runtime metadata: success (7ms)")
        );
    }

    #[test]
    fn completed_source_action_refreshes_the_catalog() {
        let original = registration("docs", "codex", "old description");
        let mut refreshed = original.clone();
        refreshed.command = Some("new-server".to_owned());
        let mut app = McpApp::new(vec![original]);

        app.finish_source_action(0, "edit", "edited", Ok(vec![refreshed]));

        assert_eq!(
            app.selected_registration()
                .and_then(|entry| entry.command.as_deref()),
            Some("new-server")
        );
        assert!(
            app.notice
                .as_ref()
                .is_some_and(|notice| !notice.error && notice.message.starts_with("edited "))
        );
    }

    #[test]
    fn completed_delete_rebuilds_indices_and_keeps_a_valid_selection() {
        let first = registration("first", "codex", "first");
        let second = registration("second", "codex", "second");
        let mut app = McpApp::new(vec![first.clone(), second]);
        app.select_next();
        app.begin_delete();

        app.finish_delete(1, Ok(vec![first]));

        assert_eq!(app.registrations.len(), 1);
        assert_eq!(app.selected_catalog_index(), Some(0));
        assert_eq!(app.pending_delete, None);
        assert!(
            app.notice.as_ref().is_some_and(
                |notice| !notice.error && notice.message == "deleted codex:user:second"
            )
        );
    }

    #[test]
    fn source_edit_and_delete_keep_comment_and_owner_boundaries_distinct() {
        let mut jsonc = registration("docs", "opencode", "docs");
        jsonc.source_format = McpSourceFormat::Jsonc;
        let mut app = McpApp::new(vec![jsonc]);

        assert_eq!(app.begin_external_edit(), Some(0));
        app.begin_delete();
        assert_eq!(app.pending_delete, None);
        assert!(app.notice.as_ref().is_some_and(|notice| notice.error));

        let mut plugin = registration("docs", "claude", "docs");
        plugin.scope = "plugin".to_owned();
        let mut app = McpApp::new(vec![plugin]);

        assert_eq!(app.begin_external_edit(), None);
        assert!(app.notice.as_ref().is_some_and(|notice| notice.error));
    }

    #[test]
    fn detail_scroll_is_not_limited_by_the_terminal_u16_offset() {
        let mut app = McpApp::new(vec![registration("large", "codex", "large catalog")]);
        let beyond_u16 = usize::from(u16::MAX) + 25;
        app.detail_max_scroll = beyond_u16;

        app.scroll_detail(isize::MAX);

        assert_eq!(app.detail_scroll, beyond_u16);
    }
}
