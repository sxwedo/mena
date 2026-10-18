use std::path::PathBuf;

use super::app::MemoriesApp;
use crate::memory::MemoryFile;

fn file(name: &str, provider: &str, scope: &str) -> MemoryFile {
    MemoryFile {
        name: name.to_owned(),
        provider: provider.to_owned(),
        scope: scope.to_owned(),
        path: PathBuf::from(format!("/{provider}/{scope}/{name}")),
        location: scope.to_owned(),
        size_bytes: 10,
    }
}

fn app() -> MemoriesApp {
    MemoriesApp::new(vec![
        file("CLAUDE.md", "claude", "user"),
        file("AGENTS.md", "codex", "user"),
        file("CLAUDE.md", "claude", "project"),
    ])
}

#[test]
fn filter_matches_name_provider_and_scope() {
    let mut memories = app();
    assert_eq!(memories.visible.len(), 3);

    memories.query = "codex".to_owned();
    memories.recompute_filter();
    assert_eq!(memories.visible.len(), 1);
    assert_eq!(memories.selected_file().unwrap().provider, "codex");

    memories.query = "claude project".to_owned();
    memories.recompute_filter();
    assert_eq!(memories.visible.len(), 1);
    assert_eq!(memories.selected_file().unwrap().scope, "project");
}

#[test]
fn delete_key_arms_confirmation_only_for_a_selected_file() {
    let mut memories = app();

    memories.begin_delete();
    assert_eq!(memories.pending_delete, Some(0));

    memories.pending_delete = None;
    memories.selected_index = memories.visible.len();
    memories.begin_delete();
    assert_eq!(memories.pending_delete, None);
}

#[test]
fn deletion_removes_the_file_and_clamps_selection() {
    let mut memories = app();
    memories.selected_index = 2;
    let removed = memories.selected_file().unwrap().path.clone();

    memories.apply_deletion(&removed);

    assert_eq!(memories.files.len(), 2);
    assert!(memories.files.iter().all(|file| file.path != removed));
    assert!(memories.visible.len() <= 2);
    assert!(memories.selected_index < memories.visible.len());
}
