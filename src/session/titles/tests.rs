use super::{MAX_TITLE_CHARS, load, normalize, persist, put};

#[test]
fn normalize_collapses_whitespace_and_clears_blank_titles() {
    assert_eq!(
        normalize("  Custom   name  ").expect("valid title"),
        Some("Custom name".to_owned())
    );
    assert_eq!(normalize("   ").expect("blank title"), None);
    assert_eq!(normalize("").expect("empty title"), None);
}

#[test]
fn normalize_rejects_titles_over_the_display_limit() {
    let allowed = "a".repeat(MAX_TITLE_CHARS);
    assert_eq!(
        normalize(&allowed).expect("limit is allowed"),
        Some(allowed)
    );
    let error = normalize(&"a".repeat(MAX_TITLE_CHARS + 1)).expect_err("over limit");
    assert!(format!("{error:#}").contains("at most 120 characters"));
}

#[test]
fn overlay_round_trips_colon_keys_and_drops_the_file_when_empty() {
    let directory = tempfile::tempdir().expect("temporary overlay directory");
    let path = directory.path().join("session-titles.toml");

    put(&path, "codex:abc-123", Some("Refactor catalog".to_owned())).expect("write overlay");
    let loaded = load(&path).expect("read overlay");
    assert_eq!(
        loaded.get("codex:abc-123").map(String::as_str),
        Some("Refactor catalog")
    );

    put(&path, "codex:abc-123", None).expect("clear overlay");
    assert!(!path.exists(), "empty overlay file should be removed");
    assert!(load(&path).expect("missing overlay").is_empty());
}

#[test]
fn persist_replaces_an_existing_overlay_atomically() {
    let directory = tempfile::tempdir().expect("temporary overlay directory");
    let path = directory.path().join("session-titles.toml");
    let mut titles = std::collections::BTreeMap::new();
    titles.insert("claude:one".to_owned(), "First".to_owned());
    persist(&path, &titles).expect("create overlay");
    titles.insert("claude:one".to_owned(), "Second".to_owned());
    persist(&path, &titles).expect("replace overlay");
    let loaded = load(&path).expect("read overlay");
    assert_eq!(loaded.get("claude:one").map(String::as_str), Some("Second"));
}
