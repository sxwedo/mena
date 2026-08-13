use std::path::PathBuf;

use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::mcp::{McpConfigPatch, McpRegistration};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum McpEditFieldKind {
    Enabled,
    Command,
    Args,
    Url,
    Cwd,
}

impl McpEditFieldKind {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Enabled => "Enabled",
            Self::Command => "Command",
            Self::Args => "Arguments (JSON array)",
            Self::Url => "URL",
            Self::Cwd => "Working directory",
        }
    }

    const fn is_toggle(self) -> bool {
        matches!(self, Self::Enabled)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct McpEditField {
    pub(crate) kind: McpEditFieldKind,
    pub(crate) value: String,
    original: String,
}

impl McpEditField {
    pub(crate) fn is_dirty(&self) -> bool {
        self.value != self.original
    }
}

pub(crate) struct McpEditForm {
    pub(crate) registration_index: usize,
    pub(crate) selector: String,
    pub(crate) source: PathBuf,
    pub(crate) fields: Vec<McpEditField>,
    pub(crate) selected: usize,
    pub(crate) editing: bool,
    pub(crate) cursor: usize,
    pub(crate) error: Option<String>,
}

pub(crate) enum McpEditAction {
    Continue,
    Cancel,
    Save(McpConfigPatch),
}

impl McpEditForm {
    pub(crate) fn new(index: usize, registration: &McpRegistration) -> Result<Self> {
        crate::mcp::ensure_basic_config_editable(registration)?;
        let mut fields = Vec::new();
        if crate::mcp::basic_config_can_toggle_enabled(registration) {
            fields.push(field(
                McpEditFieldKind::Enabled,
                if registration.enabled {
                    "true"
                } else {
                    "false"
                },
            ));
        }
        if let Some(command) = &registration.command {
            fields.push(field(McpEditFieldKind::Command, command));
            fields.push(field(
                McpEditFieldKind::Args,
                &serde_json::to_string(&registration.args)
                    .context("failed to prepare MCP arguments for editing")?,
            ));
            let cwd = registration
                .cwd
                .as_deref()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default();
            fields.push(field(McpEditFieldKind::Cwd, &cwd));
        }
        if let Some(url) = &registration.url {
            fields.push(field(McpEditFieldKind::Url, url));
        }
        if fields.is_empty() {
            anyhow::bail!(
                "MCP registration `{}` has no safely editable basic fields; press `o` to open {}",
                registration.selector,
                registration.source.display()
            );
        }
        Ok(Self {
            registration_index: index,
            selector: registration.selector.clone(),
            source: registration.source.clone(),
            fields,
            selected: 0,
            editing: false,
            cursor: 0,
            error: None,
        })
    }

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> McpEditAction {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
            return match self.patch() {
                Ok(patch) => McpEditAction::Save(patch),
                Err(error) => {
                    self.error = Some(format!("{error:#}"));
                    McpEditAction::Continue
                }
            };
        }
        if self.editing {
            return self.handle_text_key(key);
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => McpEditAction::Cancel,
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                self.selected = self.selected.saturating_sub(1);
                self.error = None;
                McpEditAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                self.selected = (self.selected + 1).min(self.fields.len().saturating_sub(1));
                self.error = None;
                McpEditAction::Continue
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                if self.selected_field().kind.is_toggle() {
                    let field = self.selected_field_mut();
                    if field.value == "true" {
                        "false"
                    } else {
                        "true"
                    }
                    .clone_into(&mut field.value);
                } else {
                    self.editing = true;
                    self.cursor = self.selected_field().value.len();
                }
                self.error = None;
                McpEditAction::Continue
            }
            _ => McpEditAction::Continue,
        }
    }

    fn handle_text_key(&mut self, key: KeyEvent) -> McpEditAction {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                self.editing = false;
            }
            KeyCode::Left => {
                self.cursor = previous_boundary(&self.selected_field().value, self.cursor);
            }
            KeyCode::Right => {
                self.cursor = next_boundary(&self.selected_field().value, self.cursor);
            }
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.selected_field().value.len(),
            KeyCode::Backspace => {
                let previous = previous_boundary(&self.selected_field().value, self.cursor);
                let cursor = self.cursor;
                self.selected_field_mut()
                    .value
                    .replace_range(previous..cursor, "");
                self.cursor = previous;
            }
            KeyCode::Delete => {
                let next = next_boundary(&self.selected_field().value, self.cursor);
                let cursor = self.cursor;
                self.selected_field_mut()
                    .value
                    .replace_range(cursor..next, "");
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.selected_field_mut().value.clear();
                self.cursor = 0;
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                let cursor = self.cursor;
                self.selected_field_mut().value.insert(cursor, character);
                self.cursor += character.len_utf8();
            }
            _ => {}
        }
        self.error = None;
        McpEditAction::Continue
    }

    fn patch(&self) -> Result<McpConfigPatch> {
        let mut patch = McpConfigPatch::default();
        for field in self.fields.iter().filter(|field| field.is_dirty()) {
            match field.kind {
                McpEditFieldKind::Enabled => patch.enabled = Some(field.value == "true"),
                McpEditFieldKind::Command => {
                    patch.command = Some(Some(field.value.clone()));
                }
                McpEditFieldKind::Args => {
                    patch.args = Some(
                        serde_json::from_str::<Vec<String>>(&field.value)
                            .context("arguments must be a JSON array of strings")?,
                    );
                }
                McpEditFieldKind::Url => patch.url = Some(Some(field.value.clone())),
                McpEditFieldKind::Cwd => {
                    patch.cwd =
                        Some((!field.value.is_empty()).then(|| PathBuf::from(&field.value)));
                }
            }
        }
        Ok(patch)
    }

    fn selected_field(&self) -> &McpEditField {
        &self.fields[self.selected]
    }

    fn selected_field_mut(&mut self) -> &mut McpEditField {
        &mut self.fields[self.selected]
    }
}

fn field(kind: McpEditFieldKind, value: &str) -> McpEditField {
    McpEditField {
        kind,
        value: value.to_owned(),
        original: value.to_owned(),
    }
}

fn previous_boundary(value: &str, cursor: usize) -> usize {
    value[..cursor]
        .char_indices()
        .next_back()
        .map_or(0, |(index, _)| index)
}

fn next_boundary(value: &str, cursor: usize) -> usize {
    value[cursor..]
        .char_indices()
        .nth(1)
        .map_or(value.len(), |(index, _)| cursor + index)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::{McpEditAction, McpEditFieldKind, McpEditForm};
    use crate::mcp::{McpRegistration, McpSourceFormat, McpTimeouts, McpToolPolicy, McpTransport};

    fn registration() -> McpRegistration {
        McpRegistration {
            selector: "codex:user:docs".to_owned(),
            name: "docs".to_owned(),
            provider: "codex".to_owned(),
            scope: "user".to_owned(),
            source: "/tmp/config.toml".into(),
            source_format: McpSourceFormat::Toml,
            transport: McpTransport::Stdio,
            enabled: true,
            valid: true,
            display_name: None,
            description: None,
            command: Some("npx".to_owned()),
            args: vec!["old".to_owned()],
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
    fn form_edits_arguments_as_a_json_string_array() {
        let mut form = McpEditForm::new(0, &registration()).expect("editable form");
        form.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        form.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        form.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        form.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        for character in r#"["new","--stdio"]"#.chars() {
            form.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        }
        let action = form.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL));

        assert!(matches!(
            action,
            McpEditAction::Save(crate::mcp::McpConfigPatch {
                args: Some(arguments),
                ..
            }) if arguments == ["new", "--stdio"]
        ));
    }

    #[test]
    fn form_hides_enable_toggle_when_the_client_has_no_native_field() {
        let mut registration = registration();
        registration.selector = "claude:user:docs".to_owned();
        registration.provider = "claude".to_owned();
        registration.source_format = McpSourceFormat::Json;

        let form = McpEditForm::new(0, &registration).expect("editable command fields");

        assert!(
            form.fields
                .iter()
                .all(|field| field.kind != McpEditFieldKind::Enabled)
        );
    }
}
