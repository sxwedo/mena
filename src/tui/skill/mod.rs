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
    mut load_entry: impl FnMut(&AgentSkill, &Path) -> Result<SkillEntry>,
) -> Result<Option<Surface>> {
    run_skill_browser(skills, &mut load_entry)
}
