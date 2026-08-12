#![allow(
    clippy::too_many_lines,
    clippy::wildcard_imports,
    clippy::redundant_pub_crate
)]

//! Responsive Terminal User Interface (TUI) components for session management,
//! skill browsing, and agent selection.

pub(crate) mod agent_launcher;
pub(crate) mod common;
pub(crate) mod mcp;
pub(crate) mod session;
pub(crate) mod skill;

#[cfg(test)]
mod tests;

pub use agent_launcher::{select_and_launch_agent, select_launch_mode_for_agent};
pub(crate) use mcp::manage_mcp;
pub(crate) use session::manage_sessions;
pub use skill::manage_skills;
