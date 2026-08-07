use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;
use std::thread;

use anyhow::{Result, bail};
use sysinfo::{ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind};

use crate::settings::CustomAgentSettings;

/// A developer-agent implementation recognized by mena.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum AgentKind {
    ClaudeCode,
    Codex,
    GeminiCli,
    OpenCode,
    Pi,
    OhMyPi,
    Cursor,
    Custom(String),
}

impl AgentKind {
    #[must_use]
    pub const fn slug(&self) -> &str {
        match self {
            Self::ClaudeCode => "claude",
            Self::Codex => "codex",
            Self::GeminiCli => "gemini",
            Self::OpenCode => "opencode",
            Self::Pi => "pi",
            Self::OhMyPi => "omp",
            Self::Cursor => "cursor",
            Self::Custom(name) => name.as_str(),
        }
    }

    #[must_use]
    pub fn from_slug(value: &str) -> Option<Self> {
        match value {
            "claude" => Some(Self::ClaudeCode),
            "codex" => Some(Self::Codex),
            "gemini" => Some(Self::GeminiCli),
            "opencode" => Some(Self::OpenCode),
            "pi" => Some(Self::Pi),
            "omp" => Some(Self::OhMyPi),
            "cursor" => Some(Self::Cursor),
            _ => None,
        }
    }

    #[must_use]
    pub fn executable_name<'a>(
        &'a self,
        custom: &'a BTreeMap<String, CustomAgentSettings>,
    ) -> Option<&'a str> {
        match self {
            Self::ClaudeCode => Some("claude"),
            Self::Codex => Some("codex"),
            Self::GeminiCli => Some("gemini"),
            Self::OpenCode => Some("opencode"),
            Self::Pi => Some("pi"),
            Self::OhMyPi => Some("omp"),
            Self::Cursor => Some("cursor-agent"),
            Self::Custom(name) => custom
                .get(name)
                .and_then(|c| c.executables.first().map(String::as_str)),
        }
    }

    #[must_use]
    pub fn is_installed(&self, custom: &BTreeMap<String, CustomAgentSettings>) -> bool {
        match self {
            Self::ClaudeCode => is_executable_in_path("claude"),
            Self::Codex => is_executable_in_path("codex"),
            Self::GeminiCli => is_executable_in_path("gemini"),
            Self::OpenCode => is_executable_in_path("opencode"),
            Self::Pi => is_executable_in_path("pi"),
            Self::OhMyPi => is_executable_in_path("omp"),
            Self::Cursor => {
                is_executable_in_path("cursor-agent") || is_executable_in_path("cursor")
            }
            Self::Custom(name) => custom
                .get(name)
                .is_some_and(|c| c.executables.iter().any(|exe| is_executable_in_path(exe))),
        }
    }

    #[must_use]
    pub fn all_kinds(custom: &BTreeMap<String, CustomAgentSettings>) -> Vec<Self> {
        let mut kinds = vec![
            Self::ClaudeCode,
            Self::Codex,
            Self::OhMyPi,
            Self::OpenCode,
            Self::Pi,
            Self::Cursor,
            Self::GeminiCli,
        ];
        for name in custom.keys() {
            kinds.push(Self::Custom(name.clone()));
        }
        kinds
    }
}

impl fmt::Display for AgentKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
            Self::GeminiCli => "Gemini CLI",
            Self::OpenCode => "OpenCode",
            Self::Pi => "Pi",
            Self::OhMyPi => "Oh My Pi",
            Self::Cursor => "Cursor",
            Self::Custom(name) => name,
        };
        formatter.write_str(label)
    }
}

/// Process information captured at one instant.
#[derive(Debug, Clone, PartialEq)]
pub struct ProcessSnapshot {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub executable: PathBuf,
    pub command: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub started_at: u64,
    pub run_time: u64,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub status: String,
}

/// A recognized live developer-agent process.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveAgent {
    pub kind: AgentKind,
    pub process: ProcessSnapshot,
}

/// Returns absolute files currently held open by a process when the host
/// exposes that information. Missing permissions or a process exit produce no
/// evidence; callers must never turn absence into a guessed association.
#[must_use]
pub fn open_file_paths(pid: u32) -> Vec<PathBuf> {
    open_file_paths_platform(pid)
}

#[cfg(target_os = "linux")]
fn open_file_paths_platform(pid: u32) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(format!("/proc/{pid}/fd")) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| std::fs::read_link(entry.path()).ok())
        .filter(|path| path.is_absolute())
        .collect()
}

#[cfg(target_os = "macos")]
fn open_file_paths_platform(pid: u32) -> Vec<PathBuf> {
    const MAX_LSOF_OUTPUT_BYTES: usize = 16 * 1_024 * 1_024;
    let Ok(output) = Command::new("/usr/sbin/lsof")
        .args(["-Fn", "-p", &pid.to_string()])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() || output.stdout.len() > MAX_LSOF_OUTPUT_BYTES {
        return Vec::new();
    }
    output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter_map(|line| line.strip_prefix(b"n"))
        .filter_map(|line| std::str::from_utf8(line).ok())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .collect()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
const fn open_file_paths_platform(_pid: u32) -> Vec<PathBuf> {
    Vec::new()
}

#[must_use]
pub fn recognize_agent(process: &ProcessSnapshot) -> Option<AgentKind> {
    let executable = process
        .executable
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match executable.as_str() {
        "claude" => return Some(AgentKind::ClaudeCode),
        "codex" => return Some(AgentKind::Codex),
        "gemini" => return Some(AgentKind::GeminiCli),
        "opencode" => return Some(AgentKind::OpenCode),
        "pi" => return Some(AgentKind::Pi),
        "omp" => return Some(AgentKind::OhMyPi),
        "cursor-agent" => return Some(AgentKind::Cursor),
        _ => {}
    }

    if !matches!(executable.as_str(), "node" | "node.exe" | "bun" | "bun.exe") {
        return None;
    }

    let command = process.command.join(" ").to_ascii_lowercase();
    if command.contains("@anthropic-ai/claude-code") {
        Some(AgentKind::ClaudeCode)
    } else if command.contains("@openai/codex") {
        Some(AgentKind::Codex)
    } else if command.contains("@google/gemini-cli") {
        Some(AgentKind::GeminiCli)
    } else if command.contains("opencode-ai") || command.contains("/opencode/") {
        Some(AgentKind::OpenCode)
    } else if command.contains("@oh-my-pi/pi-coding-agent") {
        Some(AgentKind::OhMyPi)
    } else if command.contains("pi-coding-agent") {
        Some(AgentKind::Pi)
    } else {
        None
    }
}

pub fn discover_live_agents(
    custom: &BTreeMap<String, CustomAgentSettings>,
) -> Result<Vec<LiveAgent>> {
    discover_live_agents_with_cpu(custom, false)
}

pub fn discover_live_agents_with_cpu(
    custom: &BTreeMap<String, CustomAgentSettings>,
    sample_cpu: bool,
) -> Result<Vec<LiveAgent>> {
    validate_custom_agents(custom)?;
    let refresh = ProcessRefreshKind::nothing()
        .with_cpu()
        .with_memory()
        .with_cwd(UpdateKind::OnlyIfNotSet)
        .with_cmd(UpdateKind::OnlyIfNotSet)
        .with_exe(UpdateKind::OnlyIfNotSet)
        .without_tasks();
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);
    if sample_cpu {
        thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        system.refresh_processes_specifics(ProcessesToUpdate::All, true, refresh);
    }

    let mut agents: Vec<_> = system
        .processes()
        .iter()
        .map(|(pid, process)| {
            let snapshot = snapshot(*pid, process);
            recognize_agent_with_custom(&snapshot, custom).map(|kind| {
                kind.map(|kind| LiveAgent {
                    kind,
                    process: snapshot,
                })
            })
        })
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect();
    agents.sort_by_key(|agent| agent.process.pid);
    Ok(agents)
}

fn recognize_agent_with_custom(
    process: &ProcessSnapshot,
    custom: &BTreeMap<String, CustomAgentSettings>,
) -> Result<Option<AgentKind>> {
    if let Some(kind) = recognize_agent(process) {
        return Ok(Some(kind));
    }
    let executable = process
        .executable
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let command = process.command.join("\u{0}");
    let matches: Vec<&str> = custom
        .iter()
        .filter(|(_, settings)| {
            settings
                .executables
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(executable))
                && settings
                    .command_contains
                    .iter()
                    .all(|marker| command.contains(marker))
        })
        .map(|(name, _)| name.as_str())
        .collect();
    match matches.as_slice() {
        [] => Ok(None),
        [name] => Ok(Some(AgentKind::Custom((*name).to_owned()))),
        _ => bail!(
            "process {} matches multiple custom agents: {}; make their matchers more specific",
            process.pid,
            matches.join(", ")
        ),
    }
}

pub fn validate_custom_agents(custom: &BTreeMap<String, CustomAgentSettings>) -> Result<()> {
    for (name, settings) in custom {
        if name.is_empty()
            || !name.chars().all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_')
            })
        {
            bail!(
                "invalid custom agent name `{name}`; use letters, digits, hyphens, or underscores"
            );
        }
        if AgentKind::from_slug(name).is_some() {
            bail!("custom agent name `{name}` collides with a built-in agent");
        }
        if settings.executables.is_empty() || settings.executables.iter().any(String::is_empty) {
            bail!("custom agent `{name}` must define at least one non-empty executable");
        }
        if settings.command_contains.iter().any(String::is_empty) {
            bail!("custom agent `{name}` contains an empty command matcher");
        }
        if !settings.resume.is_empty() {
            if settings.resume[0].is_empty() {
                bail!("custom agent `{name}` has an empty resume executable");
            }
            if !settings
                .resume
                .iter()
                .any(|part| part.contains("{session}"))
            {
                bail!("custom agent `{name}` resume argv must contain `{{session}}`");
            }
        }
    }
    Ok(())
}

fn snapshot(pid: sysinfo::Pid, process: &sysinfo::Process) -> ProcessSnapshot {
    let executable = process
        .exe()
        .map_or_else(|| PathBuf::from(process.name()), Path::to_path_buf);
    ProcessSnapshot {
        pid: pid.as_u32(),
        parent_pid: process.parent().map(sysinfo::Pid::as_u32),
        executable,
        command: process
            .cmd()
            .iter()
            .map(|part| part.to_string_lossy().into_owned())
            .collect(),
        cwd: process.cwd().map(Path::to_path_buf),
        started_at: process.start_time(),
        run_time: process.run_time(),
        cpu_percent: process.cpu_usage(),
        memory_bytes: process.memory(),
        status: status_label(process.status()).to_owned(),
    }
}

const fn status_label(status: ProcessStatus) -> &'static str {
    match status {
        ProcessStatus::Run | ProcessStatus::Waking => "running",
        ProcessStatus::Idle => "idle",
        ProcessStatus::Sleep
        | ProcessStatus::Wakekill
        | ProcessStatus::Parked
        | ProcessStatus::LockBlocked
        | ProcessStatus::UninterruptibleDiskSleep => "sleeping",
        ProcessStatus::Stop | ProcessStatus::Tracing | ProcessStatus::Suspended => "stopped",
        ProcessStatus::Zombie | ProcessStatus::Dead => "exited",
        ProcessStatus::Unknown(_) => "unknown",
    }
}
#[must_use]
pub fn is_executable_in_path(program: &str) -> bool {
    if program.contains('/') || program.contains('\\') {
        return Path::new(program).is_file();
    }
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(program);
            if candidate.is_file() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(meta) = candidate.metadata()
                        && meta.permissions().mode() & 0o111 != 0
                    {
                        return true;
                    }
                }
                #[cfg(not(unix))]
                return true;
            }
            #[cfg(target_os = "windows")]
            {
                let candidate_exe = dir.join(format!("{program}.exe"));
                if candidate_exe.is_file() {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use super::{AgentKind, ProcessSnapshot, recognize_agent_with_custom, validate_custom_agents};
    use crate::settings::CustomAgentSettings;

    #[test]
    fn custom_recognizers_require_executable_and_all_command_markers() {
        let custom = BTreeMap::from([(
            "my_agent".to_owned(),
            CustomAgentSettings {
                executables: vec!["node".to_owned()],
                command_contains: vec!["my-agent.js".to_owned(), "--daemon".to_owned()],
                resume: vec![
                    "my-agent".to_owned(),
                    "resume".to_owned(),
                    "{session}".to_owned(),
                ],
            },
        )]);
        validate_custom_agents(&custom).expect("valid custom agent");
        let process = ProcessSnapshot {
            pid: 7,
            parent_pid: None,
            executable: PathBuf::from("/usr/bin/node"),
            command: vec![
                "node".to_owned(),
                "my-agent.js".to_owned(),
                "--daemon".to_owned(),
            ],
            cwd: None,
            started_at: 1,
            run_time: 1,
            cpu_percent: 0.0,
            memory_bytes: 0,
            status: "sleeping".to_owned(),
        };
        assert_eq!(
            recognize_agent_with_custom(&process, &custom).expect("recognition"),
            Some(AgentKind::Custom("my_agent".to_owned()))
        );
    }

    #[test]
    fn invalid_custom_agent_config_fails_before_process_discovery() {
        let custom = BTreeMap::from([(
            "codex".to_owned(),
            CustomAgentSettings {
                executables: vec!["other".to_owned()],
                command_contains: Vec::new(),
                resume: Vec::new(),
            },
        )]);
        let error = validate_custom_agents(&custom).expect_err("built-in name collision");
        assert!(error.to_string().contains("collides with a built-in"));
    }
}
