//! A prepared argv plus ownership of any temporary context handoff.

use anyhow::{Context, Result};

use super::{DesktopLaunch, DesktopPreparedLaunch};
use crate::continuation::PreparedContinuation;
use crate::process::find_executable;
use crate::session::{AgentSession, NativeResumeCommand};
use std::path::Path;

impl DesktopPreparedLaunch {
    pub(super) fn prepare(
        command: NativeResumeCommand,
        session: &AgentSession,
        title: String,
        target: Option<String>,
        handoff: Option<PreparedContinuation>,
    ) -> Result<Self> {
        let directory = session
            .project
            .as_deref()
            .context("会话没有项目目录，无法启动 Agent。")?;
        Self::in_directory(command, directory, title, target, handoff)
    }

    pub(super) fn in_directory(
        command: NativeResumeCommand,
        directory: &Path,
        title: String,
        target: Option<String>,
        handoff: Option<PreparedContinuation>,
    ) -> Result<Self> {
        let executable = find_executable(&command.program).with_context(|| {
            format!(
                "找不到 {}。请先安装 Agent，或将可执行文件加入 PATH。",
                command.program
            )
        })?;
        anyhow::ensure!(
            directory.is_dir(),
            "项目目录已不存在：{}",
            directory.display()
        );
        Ok(Self {
            data: DesktopLaunch {
                executable: executable.display().to_string(),
                arguments: command.args,
                directory: directory.display().to_string(),
                title,
                target,
            },
            _handoff: handoff,
        })
    }
}
