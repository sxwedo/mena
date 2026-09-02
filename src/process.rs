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

#[cfg(target_os = "macos")]
fn open_file_paths_platform(pid: u32) -> Vec<PathBuf> {
    use std::io::Read;
    use std::process::Stdio;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    const MAX_LSOF_OUTPUT_BYTES: usize = 16 * 1_024 * 1_024;
    // lsof can stall for half a minute on a process holding sockets that
    // trigger blocking kernel queries (observed ~30s at 0% CPU). Evidence
    // collection must stay bounded: past the deadline the process yields no
    // evidence, and callers already treat missing evidence as "no exact
    // association", which keeps deletion protection fail-closed. A healthy
    // lsof finishes in tens of milliseconds, so half a second leaves ample
    // headroom while keeping interactive startup under a second even when a
    // stalled process burns the whole budget.
    const LSOF_DEADLINE: Duration = Duration::from_millis(500);
    const READER_GRACE: Duration = Duration::from_secs(1);

    let Ok(mut child) = Command::new("/usr/sbin/lsof")
        .args(["-Fn", "-p", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return Vec::new();
    };
    let stdout = child.stdout.take();
    // Drain stdout on a side thread so a stalled lsof that never writes
    // cannot deadlock the caller on a full pipe; the cap keeps runaway
    // output bounded.
    let collected: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let reader_slot = Arc::clone(&collected);
    let reader = thread::spawn(move || {
        let Some(mut stdout) = stdout else {
            return;
        };
        let mut chunk = [0_u8; 16 * 1_024];
        loop {
            match stdout.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => {
                    let mut buffer = reader_slot.lock().expect("lsof reader lock");
                    if buffer.len() > MAX_LSOF_OUTPUT_BYTES {
                        break;
                    }
                    buffer.extend_from_slice(&chunk[..read]);
                }
                Err(ref error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
    });
    let deadline = Instant::now() + LSOF_DEADLINE;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Vec::new();
                }
                thread::sleep(Duration::from_millis(5));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Vec::new();
            }
        }
    };
    // The child exited; its stdout closes once lsof's own helper children
    // are reaped, so wait briefly before taking the output.
    let grace_deadline = Instant::now() + READER_GRACE;
    while !reader.is_finished() && Instant::now() < grace_deadline {
        thread::sleep(Duration::from_millis(5));
    }
    let output = collected.lock().expect("lsof output lock").clone();
    if !status.success() || output.len() > MAX_LSOF_OUTPUT_BYTES {
        return Vec::new();
    }
    output
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
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use super::{
        AgentKind, ProcessSnapshot, recognize_agent_with_custom, recognize_live_agents,
        validate_custom_agents,
    };
    use crate::settings::CustomAgentSettings;

    fn process(pid: u32, executable: &str, command: &[&str]) -> ProcessSnapshot {
        ProcessSnapshot {
            pid,
            parent_pid: None,
            executable: PathBuf::from(executable),
            command: command.iter().map(ToString::to_string).collect(),
            cwd: Some(PathBuf::from("/work/project")),
            started_at: 1,
            run_time: 2,
            cpu_percent: 0.0,
            memory_bytes: 0,
            status: "sleeping".to_owned(),
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn open_file_paths_reports_absolute_paths_and_skips_missing_processes() {
        use super::open_file_paths;

        // The test binary itself holds open files, all reported absolutely.
        let paths = open_file_paths(std::process::id());
        assert!(paths.iter().all(|path| path.is_absolute()));

        // A process that cannot exist yields no evidence instead of hanging.
        assert!(open_file_paths(u32::MAX - 1).is_empty());
    }

    #[test]
    fn omp_extension_hosts_are_not_live_agents() {
        let host = process(
            11,
            "/usr/local/bin/omp",
            &[
                "omp",
                "--extension",
                "/Users/swd/.omp/agent/extensions/orca-agent-status.ts",
            ],
        );
        assert_eq!(
            recognize_agent_with_custom(&host, &BTreeMap::new()).expect("recognition"),
            None
        );

        // An interactive omp run stays a live agent.
        let interactive = process(12, "/usr/local/bin/omp", &["omp"]);
        assert_eq!(
            recognize_agent_with_custom(&interactive, &BTreeMap::new()).expect("recognition"),
            Some(AgentKind::OhMyPi)
        );
    }

    #[test]
    fn leaked_environment_arguments_do_not_impersonate_agents() {
        // macOS can append environment entries to the argv of npm-spawned
        // helpers. A `PATH` that lists an agent's install directory must not
        // turn an MCP server into that agent (which would fail-closed protect
        // the whole provider catalog).
        let mcp_server = process(
            13,
            "/opt/homebrew/bin/node",
            &[
                "npm",
                "exec",
                "@upstash/context7-mcp@latest",
                "HOME=/Users/swd",
                "PATH=/Users/swd/.local/share/mise/installs/opencode/latest:/usr/bin",
            ],
        );
        assert_eq!(
            recognize_agent_with_custom(&mcp_server, &BTreeMap::new()).expect("recognition"),
            None
        );

        // The same helper without the leaked environment stays unrecognized,
        // and a real npm-run agent is still recognized from its real argv.
        let clean = process(
            14,
            "/opt/homebrew/bin/node",
            &["npm", "exec", "@upstash/context7-mcp@latest"],
        );
        assert_eq!(
            recognize_agent_with_custom(&clean, &BTreeMap::new()).expect("recognition"),
            None
        );
        let agent = process(
            15,
            "/opt/homebrew/bin/node",
            &[
                "npm",
                "exec",
                "@anthropic-ai/claude-code",
                "HOME=/Users/swd",
            ],
        );
        assert_eq!(
            recognize_agent_with_custom(&agent, &BTreeMap::new()).expect("recognition"),
            Some(AgentKind::ClaudeCode)
        );
    }

    #[test]
    fn environment_assignment_detection_covers_common_shapes() {
        use super::is_environment_assignment;

        for assignment in [
            "HOME=/Users/swd",
            "PATH=/usr/bin:/bin",
            "_=/usr/bin/env",
            "LC_ALL=en_US.UTF-8",
        ] {
            assert!(is_environment_assignment(assignment), "{assignment}");
        }
        for argument in [
            "--extension=/tmp/x.ts",
            "@upstash/context7-mcp@latest",
            "resume",
            "abc123",
            "--resume",
            "-p",
            "pt-token-with=signs",
        ] {
            assert!(!is_environment_assignment(argument), "{argument}");
        }
    }

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
        let process = process(7, "/usr/bin/node", &["node", "my-agent.js", "--daemon"]);
        assert_eq!(
            recognize_agent_with_custom(&process, &custom).expect("recognition"),
            Some(AgentKind::Custom("my_agent".to_owned()))
        );
    }

    #[test]
    fn live_agent_snapshots_recognize_built_in_and_custom_processes() {
        let custom = BTreeMap::from([(
            "my_agent".to_owned(),
            CustomAgentSettings {
                executables: vec!["node".to_owned()],
                command_contains: vec!["my-agent.js".to_owned()],
                resume: Vec::new(),
            },
        )]);
        let agents = recognize_live_agents(
            [
                process(9, "/usr/bin/other", &["other"]),
                process(7, "/opt/bin/codex", &["codex"]),
                process(3, "/usr/bin/node", &["node", "my-agent.js"]),
            ],
            &custom,
        )
        .expect("live agent recognition");

        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].process.pid, 3);
        assert_eq!(agents[0].kind, AgentKind::Custom("my_agent".to_owned()));
        assert_eq!(agents[1].process.pid, 7);
        assert_eq!(agents[1].kind, AgentKind::Codex);
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

    #[test]
    fn grok_slug_collides_with_custom_agent_names() {
        let custom = BTreeMap::from([(
            "grok".to_owned(),
            CustomAgentSettings {
                executables: vec!["other".to_owned()],
                command_contains: Vec::new(),
                resume: Vec::new(),
            },
        )]);
        let error = validate_custom_agents(&custom).expect_err("grok name collision");
        assert!(error.to_string().contains("collides with a built-in"));
    }
}
