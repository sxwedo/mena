pub(crate) mod app;
pub(crate) mod event;
pub(crate) mod render;

use anyhow::Result;

pub use self::event::run_skill_browser;
use crate::skill::{AgentSkill, SkillDetail};

pub fn manage_skills(
    skills: Vec<AgentSkill>,
    mut load_detail: impl FnMut(&AgentSkill) -> Result<SkillDetail>,
) -> Result<()> {
    run_skill_browser(skills, &mut load_detail)
}
