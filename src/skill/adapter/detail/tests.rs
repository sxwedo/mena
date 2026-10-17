
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
