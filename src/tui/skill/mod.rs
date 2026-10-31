pub(crate) mod app;
pub(crate) mod event;
pub(crate) mod render;

use anyhow::Result;

pub use self::event::run_skill_browser;
use std::path::Path;

use crate::skill::{AgentSkill, SkillEntry};
use crate::tui::common::Surface;

pub fn manage_skills(
    skills: Vec<AgentSkill>,
    saved: &mut Option<app::SkillsApp>,
    mut load_entry: impl FnMut(&AgentSkill, &Path) -> Result<SkillEntry>,
) -> Result<Option<Surface>> {
    let app = saved.get_or_insert_with(|| app::SkillsApp::new(skills.clone()));
    if app.skills != skills {
        app.refresh_catalog(skills);
    }
    app.current_detail = None;
    app.preview_error = None;
    run_skill_browser(app, &mut load_entry)
}
