use std::collections::HashSet;
use std::path::PathBuf;

use crate::skill::{AgentSkill, SkillChildItem, SkillDetail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillFocus {
    List,
    Detail,
}

#[derive(Debug, Clone)]
pub(crate) enum SkillRow {
    /// A top-level skill entry.
    Skill {
        skill_idx: usize,
        has_children: bool,
        expanded: bool,
    },
    /// Any item inside the skill directory tree (file or directory at any depth).
    Item {
        skill_idx: usize,
        full_path: PathBuf,
        name: String,
        is_dir: bool,
        expanded: bool,
        depth: usize,
        is_last: bool,
    },
}

/// Read a directory's children (dirs first, then files, each sorted).
fn read_dir_children(dir: &std::path::Path) -> Vec<SkillChildItem> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut items: Vec<SkillChildItem> = entries
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            let name = p.file_name()?.to_str()?.to_string();
            let is_dir = p.is_dir();
            Some(SkillChildItem {
                name,
                path: p,
                is_dir,
            })
        })
        .collect();
    items.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    items
}

pub(crate) struct SkillsApp {
    pub(crate) skills: Vec<AgentSkill>,
    /// Set of expanded directory paths.
    pub(crate) expanded_dirs: HashSet<PathBuf>,
    /// Flat visible row list rebuilt by `rebuild_rows`.
    pub(crate) visible_rows: Vec<SkillRow>,
    pub(crate) selected_index: usize,
    pub(crate) search_query: String,
    pub(crate) is_searching: bool,
    pub(crate) current_detail: Option<SkillDetail>,
    /// Path of the file currently shown in the preview; used to detect stale cache.
    pub(crate) preview_path: Option<PathBuf>,
    pub(crate) preview_scroll: u16,
    pub(crate) full_screen_preview: bool,
    pub(crate) focus: SkillFocus,
    pub(crate) marquee_offset: usize,
    pub(crate) show_symlinks: bool,
}

impl SkillsApp {
    pub(crate) fn new(skills: Vec<AgentSkill>) -> Self {
        let mut app = Self {
            skills,
            expanded_dirs: HashSet::new(),
            visible_rows: Vec::new(),
            selected_index: 0,
            search_query: String::new(),
            is_searching: false,
            current_detail: None,
            preview_path: None,
            preview_scroll: 0,
            full_screen_preview: false,
            focus: SkillFocus::List,
            marquee_offset: 0,
            show_symlinks: false,
        };
        app.rebuild_rows();
        app
    }

    /// Rebuild `visible_rows` from skills, filter, and expansion state.
    pub(crate) fn rebuild_rows(&mut self) {
        let q = self.search_query.to_lowercase();
        self.visible_rows.clear();

        for (skill_idx, skill) in self.skills.iter().enumerate() {
            if !self.show_symlinks && skill.is_symlink {
                continue;
            }

            if !q.is_empty() {
                let hit = skill.name.to_lowercase().contains(&q)
                    || skill.provider.to_lowercase().contains(&q)
                    || skill.scope.to_lowercase().contains(&q)
                    || skill.location.to_lowercase().contains(&q)
                    || skill.triggers.iter().any(|t| t.to_lowercase().contains(&q))
                    || skill
                        .description
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&q);
                if !hit {
                    continue;
                }
            }

            let skill_dir = skill.path.parent().map(PathBuf::from);
            let has_children = skill_dir
                .as_ref()
                .is_some_and(|d| std::fs::read_dir(d).is_ok_and(|mut r| r.next().is_some()));
            let expanded = skill_dir
                .as_ref()
                .is_some_and(|d| self.expanded_dirs.contains(d));

            self.visible_rows.push(SkillRow::Skill {
                skill_idx,
                has_children,
                expanded,
            });

            if expanded && let Some(dir) = skill_dir {
                let children = read_dir_children(&dir);
                walk_children(
                    &mut self.visible_rows,
                    &self.expanded_dirs,
                    skill_idx,
                    &children,
                    1,
                );
            }
        }

        if self.selected_index >= self.visible_rows.len() {
            self.selected_index = self.visible_rows.len().saturating_sub(1);
        }
        self.preview_scroll = 0;
        self.marquee_offset = 0;
        self.current_detail = None;
        self.preview_path = None;
    }
}

/// Recursively walk children, adding visible rows for expanded directories.
fn walk_children(
    rows: &mut Vec<SkillRow>,
    expanded_dirs: &HashSet<PathBuf>,
    skill_idx: usize,
    children: &[SkillChildItem],
    depth: usize,
) {
    let total = children.len();
    for (i, child) in children.iter().enumerate() {
        let is_last = i + 1 == total;
        let expanded = child.is_dir && expanded_dirs.contains(&child.path);

        rows.push(SkillRow::Item {
            skill_idx,
            full_path: child.path.clone(),
            name: child.name.clone(),
            is_dir: child.is_dir,
            expanded,
            depth,
            is_last,
        });

        if expanded {
            let sub = read_dir_children(&child.path);
            walk_children(rows, expanded_dirs, skill_idx, &sub, depth + 1);
        }
    }
}

impl SkillsApp {
    /// Returns the filesystem path for the currently selected row.
    pub(crate) fn selected_preview_path(&self) -> Option<PathBuf> {
        let row = self.visible_rows.get(self.selected_index)?;
        match row {
            SkillRow::Skill { skill_idx, .. } => Some(self.skills[*skill_idx].path.clone()),
            SkillRow::Item { full_path, .. } => Some(full_path.clone()),
        }
    }

    /// Returns the directory path to open for the currently selected row.
    /// If the selected item is a file (e.g. `SKILL.md`), returns its parent directory.
    pub(crate) fn selected_open_path(&self) -> Option<PathBuf> {
        let path = self.selected_preview_path()?;
        if path.is_dir() {
            Some(path)
        } else if let Some(parent) = path.parent() {
            Some(parent.to_path_buf())
        } else {
            Some(path)
        }
    }

    /// The directory path that controls expansion for the current row (if any).
    fn selected_dir_path(&self) -> Option<PathBuf> {
        let row = self.visible_rows.get(self.selected_index)?;
        match row {
            SkillRow::Skill {
                skill_idx,
                has_children: true,
                ..
            } => self.skills[*skill_idx].path.parent().map(PathBuf::from),
            SkillRow::Item {
                full_path, is_dir, ..
            } if *is_dir => Some(full_path.clone()),
            _ => None,
        }
    }

    pub(crate) fn toggle_expand(&mut self) {
        let Some(dir_path) = self.selected_dir_path() else {
            return;
        };
        if self.expanded_dirs.contains(&dir_path) {
            self.expanded_dirs.remove(&dir_path);
        } else {
            self.expanded_dirs.insert(dir_path);
        }
        self.rebuild_rows();
    }

    pub(crate) fn collapse_current(&mut self) {
        let Some(row) = self.visible_rows.get(self.selected_index).cloned() else {
            return;
        };
        match row {
            SkillRow::Skill {
                skill_idx,
                expanded: true,
                ..
            } => {
                if let Some(dir) = self.skills[skill_idx].path.parent() {
                    self.expanded_dirs.remove(dir);
                    self.rebuild_rows();
                }
            }
            SkillRow::Item {
                full_path,
                is_dir: true,
                expanded: true,
                ..
            } => {
                self.expanded_dirs.remove(&full_path);
                self.rebuild_rows();
            }
            SkillRow::Item {
                skill_idx, depth, ..
            } => {
                // On a file/deeper item: collapse the nearest expanded ancestor dir
                // and move selection to it.
                let target_depth = depth.saturating_sub(1);
                self.collapse_ancestor(skill_idx, target_depth);
            }
            SkillRow::Skill { .. } => {}
        }
    }

    /// Find the expanded ancestor at `target_depth` for `skill_idx`,
    /// collapse it, and move selection to that row.
    fn collapse_ancestor(&mut self, skill_idx: usize, target_depth: usize) {
        // Scan visible_rows for the ancestor Item (same skill, same depth) or the Skill row.
        let mut found_idx: Option<usize> = None;
        let mut collapse_path: Option<PathBuf> = None;

        for (i, r) in self.visible_rows.iter().enumerate() {
            match r {
                SkillRow::Skill {
                    skill_idx: si,
                    expanded: true,
                    ..
                } if *si == skill_idx && target_depth == 0 => {
                    found_idx = Some(i);
                    collapse_path = self.skills[skill_idx].path.parent().map(PathBuf::from);
                    break;
                }
                SkillRow::Item {
                    skill_idx: si,
                    full_path,
                    depth: d,
                    expanded: true,
                    ..
                } if *si == skill_idx && *d == target_depth => {
                    found_idx = Some(i);
                    collapse_path = Some(full_path.clone());
                    break;
                }
                _ => {}
            }
        }

        if let Some(path) = collapse_path {
            self.expanded_dirs.remove(&path);
            self.rebuild_rows();
        }
        if let Some(i) = found_idx
            && i < self.visible_rows.len()
        {
            self.selected_index = i;
        }
    }

    pub(crate) const fn select_next(&mut self) {
        if !self.visible_rows.is_empty() {
            let max_idx = self.visible_rows.len() - 1;
            if self.selected_index < max_idx {
                self.selected_index += 1;
            }
        }
    }

    pub(crate) const fn select_prev(&mut self) {
        if !self.visible_rows.is_empty() && self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }
}
