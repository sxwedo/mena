use std::collections::BTreeMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, bail};

const MAX_SKILL_BYTES: u64 = 8 * 1_024 * 1_024;

/// Frontmatter metadata extracted from a Markdown skill file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillFrontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
    pub triggers: Vec<String>,
    pub extra: BTreeMap<String, String>,
}

/// Parsed detail of a skill file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSkillDetail {
    pub frontmatter: SkillFrontmatter,
    pub content: String,
}

/// Parse skill detail from a file path.
///
/// # Errors
///
/// Returns an error if reading the file fails.
pub fn parse_skill_detail(path: &Path) -> Result<RawSkillDetail> {
    let content = read_skill_text(path)?;
    let frontmatter = parse_frontmatter(&content);
    Ok(RawSkillDetail {
        frontmatter,
        content,
    })
}

pub fn read_skill_text(path: &Path) -> Result<String> {
    let file = File::open(path)
        .with_context(|| format!("failed to open skill file {}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(MAX_SKILL_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read skill file {}", path.display()))?;
    if bytes.len() as u64 > MAX_SKILL_BYTES {
        bail!(
            "skill file {} exceeds the {} MiB preview limit",
            path.display(),
            MAX_SKILL_BYTES / 1_024 / 1_024
        );
    }
    String::from_utf8(bytes)
        .with_context(|| format!("skill file {} is not valid UTF-8", path.display()))
}

/// Parse frontmatter key-value pairs bounded by `---` lines.
#[must_use]
pub fn parse_frontmatter(content: &str) -> SkillFrontmatter {
    let mut result = SkillFrontmatter::default();
    let lines: Vec<&str> = content.lines().collect();
    let frontmatter_end = lines
        .first()
        .is_some_and(|line| line.trim() == "---")
        .then(|| {
            lines
                .iter()
                .enumerate()
                .skip(1)
                .find_map(|(index, line)| (line.trim() == "---").then_some(index))
        })
        .flatten();
    let body_start = frontmatter_end.map_or(0, |index| index + 1);

    if let Some(end) = frontmatter_end {
        let mut current_key: Option<String> = None;
        for line in &lines[1..end] {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            let is_indent = line.starts_with("  ") || line.starts_with('\t');

            if !is_indent && let Some((key, value)) = line.split_once(':') {
                let k = key.trim().to_lowercase();
                let v = unquote(value.trim());

                if v.is_empty() || v == ">" || v == "|" {
                    current_key = Some(k);
                } else {
                    current_key = Some(k.clone());
                    assign_kv(&mut result, &k, v, false);
                }
            } else if let Some(k) = &current_key {
                assign_kv(
                    &mut result,
                    k,
                    trimmed.strip_prefix("- ").unwrap_or(trimmed),
                    true,
                );
            }
        }
    }

    if result.description.is_none() {
        for line in &lines[body_start..] {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed == "---" {
                continue;
            }
            result.description = Some(trimmed.to_string());
            break;
        }
    }

    result
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(value)
}

fn assign_kv(fm: &mut SkillFrontmatter, key: &str, value: &str, append: bool) {
    match key {
        "name" => {
            if append {
                if let Some(name) = &mut fm.name {
                    name.push(' ');
                    name.push_str(value);
                } else {
                    fm.name = Some(value.to_string());
                }
            } else {
                fm.name = Some(value.to_string());
            }
        }
        "description" | "desc" => {
            if append {
                if let Some(desc) = &mut fm.description {
                    if !desc.is_empty() && desc != ">" && desc != "|" {
                        desc.push(' ');
                        desc.push_str(value);
                    } else {
                        *desc = value.to_string();
                    }
                } else {
                    fm.description = Some(value.to_string());
                }
            } else {
                fm.description = Some(value.to_string());
            }
        }
        "trigger" | "triggers" => {
            let parts: Vec<String> = value
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if append {
                fm.triggers.extend(parts);
            } else {
                fm.triggers = parts;
            }
        }
        _ => {
            if append {
                if let Some(existing) = fm.extra.get_mut(key) {
                    existing.push(' ');
                    existing.push_str(value);
                } else {
                    fm.extra.insert(key.to_string(), value.to_string());
                }
            } else {
                fm.extra.insert(key.to_string(), value.to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiline_yaml_frontmatter() {
        let text = "---\nname: ponytail\ndescription: >\n  Forces the laziest solution that actually works, simplest, shortest, most\n  minimal. Channels a senior dev who has seen everything...\ntriggers: ponytail, lazy, yagni\n---\n# Content";
        let fm = parse_frontmatter(text);
        assert_eq!(fm.name.as_deref(), Some("ponytail"));
        assert!(
            fm.description
                .as_ref()
                .unwrap()
                .contains("Forces the laziest solution")
        );
        assert!(
            fm.description
                .as_ref()
                .unwrap()
                .contains("Channels a senior dev")
        );
    }
    #[test]
    fn parses_frontmatter_correctly() {
        let text = r#"---
name: ponytail
description: "Forces the laziest solution that actually works"
triggers: ponytail, lazy, yagni
---
# Skill title
Body text here...
"#;
        let fm = parse_frontmatter(text);
        assert_eq!(fm.name.as_deref(), Some("ponytail"));
        assert_eq!(
            fm.description.as_deref(),
            Some("Forces the laziest solution that actually works")
        );
        assert_eq!(fm.triggers, vec!["ponytail", "lazy", "yagni"]);
    }

    #[test]
    fn falls_back_to_first_prose_line() {
        let text = "# My Skill\n\nThis is a cool skill description.\n\nMore details.";
        let fm = parse_frontmatter(text);
        assert_eq!(
            fm.description.as_deref(),
            Some("This is a cool skill description.")
        );
    }

    #[test]
    fn fallback_ignores_frontmatter_fields() {
        let fm = parse_frontmatter("---\nname: compact\n---\n# Compact\n\nBody description.");
        assert_eq!(fm.name.as_deref(), Some("compact"));
        assert_eq!(fm.description.as_deref(), Some("Body description."));
    }

    #[test]
    fn parses_trigger_lists_without_yaml_markers() {
        let fm = parse_frontmatter("---\ntriggers:\n  - review\n  - refactor\n---\nBody");
        assert_eq!(fm.triggers, ["review", "refactor"]);
    }
}
