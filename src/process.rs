use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::thread;

use anyhow::{Result, bail};
use sysinfo::{ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System, UpdateKind};

use crate::settings::CustomAgentSettings;

/// A developer-agent implementation recognized by mena.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum AgentKind {
    ClaudeCode,
    Codex,
    Goose,
    GeminiCli,
    OpenCode,
    Pi,
    OhMyPi,
    Cursor,
    Grok,
    Custom(String),
}

impl AgentKind {
    #[must_use]
    pub const fn slug(&self) -> &str {
        match self {
            Self::ClaudeCode => "claude",
            Self::Codex => "codex",
            Self::Goose => "goose",
            Self::GeminiCli => "gemini",
            Self::OpenCode => "opencode",
            Self::Pi => "pi",
            Self::OhMyPi => "omp",
            Self::Cursor => "cursor",
            Self::Grok => "grok",
            Self::Custom(name) => name.as_str(),
        }
    }

    #[must_use]
    pub fn from_slug(value: &str) -> Option<Self> {
        match value {
            "claude" => Some(Self::ClaudeCode),
            "codex" => Some(Self::Codex),
            "goose" => Some(Self::Goose),
            "gemini" => Some(Self::GeminiCli),
            "opencode" => Some(Self::OpenCode),
            "pi" => Some(Self::Pi),
            "omp" => Some(Self::OhMyPi),
            "cursor" => Some(Self::Cursor),
            "grok" => Some(Self::Grok),
            _ => None,
        }
    }

    #[must_use]
    pub const fn homepage_url(&self) -> &'static str {
        match self {
            Self::ClaudeCode => "https://docs.anthropic.com/en/docs/agents-and-tools/claude-code",
            Self::Codex => "https://github.com/openai/codex",
            Self::Goose => "https://block.github.io/goose/",
            Self::GeminiCli => "https://github.com/google-gemini/gemini-cli",
            Self::OpenCode => "https://opencode.ai",
            Self::Pi => "https://github.com/pi-agent",
            Self::OhMyPi => "https://github.com/sxwedo/oh-my-pi",
            Self::Cursor => "https://cursor.com",
            Self::Grok => "https://docs.x.ai/build/cli/reference",
            Self::Custom(_) => "",
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
            Self::Goose => Some("goose"),
            Self::GeminiCli => Some("gemini"),
            Self::OpenCode => Some("opencode"),
            Self::Pi => Some("pi"),
            Self::OhMyPi => Some("omp"),
            Self::Cursor => Some("cursor-agent"),
            Self::Grok => Some("grok"),
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
            Self::Goose => is_executable_in_path("goose"),
            Self::GeminiCli => is_executable_in_path("gemini"),
            Self::OpenCode => is_executable_in_path("opencode"),
            Self::Pi => is_executable_in_path("pi"),
            Self::OhMyPi => is_executable_in_path("omp"),
            Self::Cursor => {
                is_executable_in_path("cursor-agent") || is_executable_in_path("cursor")
            }
            Self::Grok => is_executable_in_path("grok") || grok_managed_binary().is_some(),
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
            Self::Grok,
            Self::Goose,
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
            Self::Goose => "Goose",
            Self::GeminiCli => "Gemini CLI",
            Self::OpenCode => "OpenCode",
            Self::Pi => "Pi",
            Self::OhMyPi => "Oh My Pi",
            Self::Cursor => "Cursor",
            Self::Grok => "Grok",
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

/// libproc declarations used to read a process's fd table directly.
///
/// `lsof` was replaced by these calls because it can stall for half a minute
/// (0% CPU, blocking kernel queries) on processes holding certain sockets,
/// which froze session protection. `proc_pidinfo` and `proc_pidfdinfo` are
/// plain syscalls with no network resolution — the same calls lsof itself
/// uses — and return immediately.
#[cfg(target_os = "macos")]
mod fd_paths {
    use std::mem::size_of;
    use std::path::PathBuf;

    /// `struct proc_fdlistfd` from `<sys/proc_info.h>`.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct ProcFdListFd {
        fd: i32,
        fd_type: u8,
    }

    const _: () = assert!(size_of::<ProcFdListFd>() == 8);

    /// `struct vnode_fdinfowithpath`, the flavor
    /// `PROC_PIDFDVNODEPATHINFO` writes. Only the trailing NUL-terminated
    /// path is read; the leading `struct proc_fileinfo` (24 bytes) plus
    /// `struct vnode_info` (152 bytes) are kept as opaque padding.
    #[repr(C, align(8))]
    struct VnodeFdInfoWithPath {
        prefix: [u8; 176],
        path: [u8; 1024],
    }

    const _: () = assert!(size_of::<VnodeFdInfoWithPath>() == 1200);

    #[link(name = "proc")]
    unsafe extern "C" {
        /// Public libproc call; the `PROC_PIDLISTFDS` flavor lists the fd
        /// table. Returns the number of bytes written, or the number needed
        /// when the buffer is too small; negative on error (e.g. no such
        /// process).
        fn proc_pidinfo(pid: i32, flavor: i32, arg: u64, buffer: *mut u8, buffersize: i32) -> i32;
        /// Returns the number of bytes written for one descriptor.
        fn proc_pidfdinfo(pid: i32, fd: i32, flavor: i32, buffer: *mut u8, buffersize: i32) -> i32;
    }

    const PROX_FDTYPE_VNODE: u8 = 1;
    const PROC_PIDLISTFDS: i32 = 1;
    const PROC_PIDFDVNODEPATHINFO: i32 = 2;
    /// Initial fd-table capacity; the call reports the needed size beyond it.
    const LISTING_ENTRIES: usize = 512;
    /// Defensive ceiling; dropping entries past it only yields less
    /// evidence, which callers treat fail-closed.
    const MAX_LISTED_FDS: usize = 50_000;

    pub(super) fn open_paths(pid: u32) -> Vec<PathBuf> {
        let Ok(pid) = i32::try_from(pid) else {
            return Vec::new();
        };
        let Ok(listing_len) = i32::try_from(LISTING_ENTRIES * size_of::<ProcFdListFd>()) else {
            return Vec::new();
        };
        let mut listing = vec![ProcFdListFd { fd: 0, fd_type: 0 }; LISTING_ENTRIES];
        // SAFETY: `listing` is valid for writes of `listing_len` bytes, the
        // exact length passed to the call.
        let bytes = unsafe {
            proc_pidinfo(
                pid,
                PROC_PIDLISTFDS,
                0,
                listing.as_mut_ptr().cast(),
                listing_len,
            )
        };
        if bytes <= 0 {
            return Vec::new();
        }
        let Ok(written) = usize::try_from(bytes) else {
            return Vec::new();
        };
        let listed = if written > listing.len() {
            // The call reports the required size when the buffer is short.
            let needed = written / size_of::<ProcFdListFd>();
            listing.resize(needed, ProcFdListFd { fd: 0, fd_type: 0 });
            let Ok(grown_len) = i32::try_from(listing.len() * size_of::<ProcFdListFd>()) else {
                return Vec::new();
            };
            // SAFETY: `listing` was just resized to cover `grown_len` bytes.
            unsafe {
                proc_pidinfo(
                    pid,
                    PROC_PIDLISTFDS,
                    0,
                    listing.as_mut_ptr().cast(),
                    grown_len,
                )
            }
        } else {
            bytes
        };
        if listed <= 0 {
            return Vec::new();
        }
        let Ok(available) = usize::try_from(listed) else {
            return Vec::new();
        };
        let count = (available / size_of::<ProcFdListFd>()).min(MAX_LISTED_FDS);

        let Ok(info_size) = i32::try_from(size_of::<VnodeFdInfoWithPath>()) else {
            return Vec::new();
        };
        let mut info = VnodeFdInfoWithPath {
            prefix: [0; 176],
            path: [0; 1024],
        };
        let mut paths = Vec::new();
        for entry in &listing[..count] {
            if entry.fd_type != PROX_FDTYPE_VNODE {
                continue;
            }
            // SAFETY: `info` is a `vnode_fdinfowithpath` buffer of exactly
            // `info_size` bytes, the flavor's documented size.
            let written = unsafe {
                proc_pidfdinfo(
                    pid,
                    entry.fd,
                    PROC_PIDFDVNODEPATHINFO,
                    (&raw mut info).cast(),
                    info_size,
                )
            };
            if written != info_size {
                // Descriptor closed or layout mismatch: no evidence for it.
                continue;
            }
            let Some(end) = info.path.iter().position(|byte| *byte == 0) else {
                continue;
            };
            if end == 0 {
                continue;
            }
            if let Ok(text) = std::str::from_utf8(&info.path[..end])
                && let path = PathBuf::from(text)
                && path.is_absolute()
            {
                paths.push(path);
            }
        }
        paths
    }
}

#[cfg(target_os = "macos")]
fn open_file_paths_platform(pid: u32) -> Vec<PathBuf> {
    fd_paths::open_paths(pid)
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

    // OMP keeps long-lived worker daemons (`__omp_worker_*`, e.g. the broker
    // and the LSP mux) running after sessions close. They hold no coding
    // session, so matching them as live agents would fail-closed protect the
    // whole OMP catalog forever. They are helpers, not interactive agents.
    if process
        .command
        .iter()
        .skip(1)
        .any(|argument| argument.starts_with("__omp_worker_"))
    {
        return None;
    }

    // OMP extension hosts (`omp --extension <script>`, e.g. status-bar
    // extensions) also outlive interactive sessions. Like the worker daemons
    // above they are helpers: matching them would protect the whole OMP
    // catalog for the extension's lifetime, and their open sockets can even
    // stall open-file evidence collection.
    if executable == "omp"
        && process
            .command
            .iter()
            .skip(1)
            .any(|argument| argument == "--extension" || argument.starts_with("--extension="))
    {
        return None;
    }

    match executable.as_str() {
        "claude" => return Some(AgentKind::ClaudeCode),
        "codex" => return Some(AgentKind::Codex),
        "goose" => return Some(AgentKind::Goose),
        "gemini" => return Some(AgentKind::GeminiCli),
        "opencode" => return Some(AgentKind::OpenCode),
        "pi" => return Some(AgentKind::Pi),
        "omp" => return Some(AgentKind::OhMyPi),
        "cursor-agent" => return Some(AgentKind::Cursor),
        "grok" => return Some(AgentKind::Grok),
        _ => {}
    }

    if !matches!(executable.as_str(), "node" | "node.exe" | "bun" | "bun.exe") {
        return None;
    }

    // macOS can append environment entries to the reported argv (observed
    // with npm-spawned helpers). A leaked `PATH` that lists an agent's install
    // directory must not turn an unrelated helper into that agent.
    let command = process
        .command
        .iter()
        .filter(|argument| !is_environment_assignment(argument))
        .cloned()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase();
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

/// Whether one reported argument looks like a leaked `NAME=value` environment
/// entry rather than a real argv element. macOS can append environment
/// entries to the arguments of npm-spawned helpers, so evidence matching and
/// display both drop these before use.
fn is_environment_assignment(argument: &str) -> bool {
    let Some((name, _)) = argument.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name.starts_with(|first: char| first.is_ascii_alphabetic() || first == '_')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
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

    recognize_live_agents(
        system
            .processes()
            .iter()
            .map(|(pid, process)| snapshot(*pid, process)),
        custom,
    )
}

fn recognize_live_agents(
    snapshots: impl IntoIterator<Item = ProcessSnapshot>,
    custom: &BTreeMap<String, CustomAgentSettings>,
) -> Result<Vec<LiveAgent>> {
    let mut agents: Vec<_> = snapshots
        .into_iter()
        .map(|snapshot| {
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
            // Drop environment entries macOS can append to argv so neither
            // evidence parsing nor display leaks `PATH` and friends.
            .filter(|argument| !is_environment_assignment(argument))
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
fn grok_managed_binary() -> Option<PathBuf> {
    let home = match std::env::var_os("GROK_HOME") {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => dirs::home_dir()?.join(".grok"),
    };
    let candidate = home.join("bin/grok");
    candidate.is_file().then_some(candidate)
}

fn is_executable_in_path(program: &str) -> bool {
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
mod tests;
