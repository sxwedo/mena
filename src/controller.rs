use std::io::{self, IsTerminal};
use std::process::Command;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::mcp::{McpCatalog, McpProbeStatus};
use crate::process::{AgentKind, LiveAgent, discover_live_agents};
use crate::session::{AgentSession, NativeResumeCommand, SessionCatalog, native_resume_command};
use crate::settings::{CustomAgentSettings, Settings};
use crate::skill::SkillCatalog;
use crate::tui;
pub use crate::ui;
use crate::view::{
    render_mcp_detail, render_mcp_table, render_process_table, render_session_table,
    render_skill_detail, render_skill_table,
};
use crate::{
    AgentLaunchArgs, McpArgs, McpSubcommand, PsArgs, SessionsArgs, SkillSubcommand, SkillsArgs,
};

/// Execute `mena mcp` without contacting any configured server unless an
/// inspect request includes `--probe` or the user explicitly presses `p` in
/// the interactive browser.
///
/// # Errors
///
/// Returns an error when a configuration is invalid, a selector is ambiguous,
/// or a requested live probe cannot be performed.
pub fn run_mcp(args: &McpArgs) -> Result<()> {
    let home = dirs::home_dir();
    let current_dir = std::env::current_dir().context("could not resolve current directory")?;
    let catalog = Arc::new(McpCatalog::scan(home.as_deref(), Some(&current_dir))?);
    match &args.command {
        Some(McpSubcommand::Inspect {
            name,
            probe,
            timeout,
            json,
        }) => {
            let detail = if *probe {
                catalog.inspect_with_probe(
                    name,
                    args.provider.as_deref(),
                    args.scope.as_deref(),
                    args.source.as_deref(),
                    *timeout,
                )?
            } else {
                catalog.inspect(
                    name,
                    args.provider.as_deref(),
                    args.scope.as_deref(),
                    args.source.as_deref(),
                )?
            };
            let probe_failure = detail.probe.as_ref().and_then(|probe| {
                (!matches!(
                    probe.status,
                    McpProbeStatus::Success | McpProbeStatus::Partial
                ))
                .then(|| {
                    probe
                        .error
                        .clone()
                        .unwrap_or_else(|| probe.status.to_string())
                })
            });
            if *json || args.json {
                println!("{}", serde_json::to_string_pretty(&detail)?);
            } else {
                print!("{}", render_mcp_detail(&detail));
            }
            if let Some(error) = probe_failure {
                bail!("live MCP probe did not succeed: {error}");
            }
        }
        Some(McpSubcommand::Open { name }) => {
            if args.json {
                bail!("--json cannot be used with `mena mcp open`");
            }
            let detail = catalog.inspect(
                name,
                args.provider.as_deref(),
                args.scope.as_deref(),
                args.source.as_deref(),
            )?;
            crate::editor::open_file(&detail.registration.source)?;
            ui::success(format!(
                "opened MCP config {}",
                detail.registration.source.display()
            ));
        }
        None => {
            let registrations = catalog.select(
                args.provider.as_deref(),
                args.scope.as_deref(),
                args.source.as_deref(),
            )?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&registrations)?);
            } else if registrations.is_empty() {
                ui::info("no MCP registrations discovered");
            } else if io::stdin().is_terminal() && io::stdout().is_terminal() {
                let registrations = registrations.into_iter().cloned().collect();
                let probe_catalog = Arc::clone(&catalog);
                let update_catalog = Arc::clone(&catalog);
                tui::manage_mcp(
                    registrations,
                    move |registration| {
                        probe_catalog.inspect_current_registration_with_probe(registration, 10)
                    },
                    move |registration, patch| {
                        update_catalog.update_basic_config(registration, patch)
                    },
                )?;
            } else {
                print!("{}", render_mcp_table(&registrations));
            }
        }
    }
    Ok(())
}

pub fn run_agent(args: &AgentLaunchArgs, settings: &Settings) -> Result<()> {
    let cwd = std::env::current_dir().context("could not resolve current working directory")?;
    let catalog = scan_sessions(None)?;
    let custom = &settings.agent.custom;

    let cwd_sessions: Vec<AgentSession> = catalog
        .sessions()
        .iter()
        .filter(|session| {
            session
                .project
                .as_deref()
                .is_some_and(|project| crate::session::paths_equivalent(project, &cwd))
        })
        .cloned()
        .collect();

    if let Some(ref provider_slug) = args.provider {
        let kind = resolve_agent_kind(provider_slug, custom)?;
        launch_agent_with_options(&kind, args, &cwd_sessions, custom)?;
    } else if io::stdin().is_terminal() && io::stdout().is_terminal() {
        let choice = tui::select_and_launch_agent(custom, &cwd_sessions)?;
        if let Some(spec) = choice {
            execute_launch(&spec)?;
        }
    } else {
        print_agent_launch_help(custom, &cwd_sessions);
    }
    Ok(())
}

/// Print a one-shot, read-only snapshot of recognized agent processes.
///
/// # Errors
///
/// Returns an error when process discovery or JSON serialization fails.
pub fn run_ps(args: &PsArgs, settings: &Settings) -> Result<()> {
    let agents = discover_live_agents(&settings.agent.custom)?;
    if args.json {
        let values: Vec<_> = agents
            .iter()
            .map(|agent| process_list_json(agent, args.verbose))
            .collect();
        println!("{}", serde_json::to_string_pretty(&values)?);
    } else {
        print!("{}", render_process_table(&agents, args.verbose));
    }
    Ok(())
}

fn resolve_agent_kind(
    slug: &str,
    custom: &std::collections::BTreeMap<String, CustomAgentSettings>,
) -> Result<AgentKind> {
    if let Some(kind) = AgentKind::from_slug(slug) {
        Ok(kind)
    } else if custom.contains_key(slug) {
        Ok(AgentKind::Custom(slug.to_owned()))
    } else {
        bail!(
            "unsupported agent provider `{slug}`; available providers: claude, codex, goose, omp, opencode, pi, cursor, gemini{}",
            if custom.is_empty() {
                String::new()
            } else {
                format!(
                    ", {}",
                    custom.keys().cloned().collect::<Vec<_>>().join(", ")
                )
            }
        );
    }
}

pub fn fresh_launch_spec(
    kind: &AgentKind,
    custom: &std::collections::BTreeMap<String, CustomAgentSettings>,
) -> Result<NativeResumeCommand> {
    match kind {
        AgentKind::ClaudeCode => Ok(NativeResumeCommand {
            program: "claude".to_owned(),
            args: Vec::new(),
        }),
        AgentKind::Codex => Ok(NativeResumeCommand {
            program: "codex".to_owned(),
            args: Vec::new(),
        }),
        AgentKind::Goose => Ok(NativeResumeCommand {
            program: "goose".to_owned(),
            args: Vec::new(),
        }),
        AgentKind::GeminiCli => Ok(NativeResumeCommand {
            program: "gemini".to_owned(),
            args: Vec::new(),
        }),
        AgentKind::OpenCode => Ok(NativeResumeCommand {
            program: "opencode".to_owned(),
            args: Vec::new(),
        }),
        AgentKind::Pi => Ok(NativeResumeCommand {
            program: "pi".to_owned(),
            args: Vec::new(),
        }),
        AgentKind::OhMyPi => Ok(NativeResumeCommand {
            program: "omp".to_owned(),
            args: Vec::new(),
        }),
        AgentKind::Cursor => Ok(NativeResumeCommand {
            program: "cursor-agent".to_owned(),
            args: Vec::new(),
        }),
        AgentKind::Custom(name) => {
            let spec = custom
                .get(name)
                .with_context(|| format!("custom agent `{name}` not found in configuration"))?;
            let program = spec
                .executables
                .first()
                .with_context(|| format!("custom agent `{name}` defines no executables"))?;
            Ok(NativeResumeCommand {
                program: program.clone(),
                args: Vec::new(),
            })
        }
    }
}

pub fn resume_launch_spec(
    kind: &AgentKind,
    session_id: &str,
    custom: &std::collections::BTreeMap<String, CustomAgentSettings>,
) -> Result<NativeResumeCommand> {
    match kind {
        AgentKind::Custom(name) => {
            let spec = custom
                .get(name)
                .with_context(|| format!("custom agent `{name}` not found in configuration"))?;
            custom_resume_spec(name, spec, session_id)
        }
        _ => native_resume_command(kind, session_id),
    }
}

fn launch_agent_with_options(
    kind: &AgentKind,
    args: &AgentLaunchArgs,
    cwd_sessions: &[AgentSession],
    custom: &std::collections::BTreeMap<String, CustomAgentSettings>,
) -> Result<()> {
    let matching_sessions: Vec<&AgentSession> = cwd_sessions
        .iter()
        .filter(|session| session.kind == *kind)
        .collect();

    let spec = if args.fresh {
        fresh_launch_spec(kind, custom)?
    } else if let Some(ref session_id) = args.session {
        resume_launch_spec(kind, session_id, custom)?
    } else if args.resume {
        let latest = matching_sessions.first().with_context(|| {
            format!(
                "no saved session for `{}` found in current directory",
                kind.slug()
            )
        })?;
        resume_launch_spec(kind, &latest.id, custom)?
    } else if matching_sessions.is_empty() {
        fresh_launch_spec(kind, custom)?
    } else if io::stdin().is_terminal() && io::stdout().is_terminal() {
        if let Some(chosen) = tui::select_launch_mode_for_agent(kind, custom, &matching_sessions)? {
            chosen
        } else {
            return Ok(());
        }
    } else {
        let latest = matching_sessions.first().unwrap();
        resume_launch_spec(kind, &latest.id, custom)?
    };

    execute_launch(&spec)
}

pub fn execute_launch(spec: &NativeResumeCommand) -> Result<()> {
    ui::info(format!("launching `{}`", spec.program));
    let mut command = Command::new(&spec.program);
    command.args(&spec.args);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = command.exec();
        bail!("failed to exec `{}`: {err}", spec.program);
    }

    #[cfg(not(unix))]
    {
        let status = command.status().with_context(|| {
            format!(
                "failed to start `{}`; install it or ensure it is available on PATH",
                spec.program
            )
        })?;
        if !status.success() {
            bail!("{} exited with status {status}", spec.program);
        }
        Ok(())
    }
}
pub fn open_url(url: &str) -> Result<()> {
    if url.is_empty() {
        bail!("no URL associated with this agent");
    }
    ui::info(format!("opening homepage: {url}"));
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut cmd = Command::new("cmd");
        cmd.args(["/c", "start", ""]);
        cmd
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut command = Command::new("xdg-open");

    command.arg(url);
    let status = command
        .status()
        .with_context(|| format!("failed to open URL `{url}`"))?;
    if !status.success() {
        bail!("browser command exited with status {status}");
    }
    Ok(())
}

fn print_agent_launch_help(
    custom: &std::collections::BTreeMap<String, CustomAgentSettings>,
    cwd_sessions: &[AgentSession],
) {
    println!("Available developer agents for current directory:\n");
    for kind in AgentKind::all_kinds(custom) {
        let slug = kind.slug();
        let installed = kind.is_installed(custom);
        let status = if installed {
            "[installed]"
        } else {
            "[not in PATH]"
        };
        let count = cwd_sessions.iter().filter(|s| s.kind == kind).count();
        let session_info = if count > 0 {
            format!("{count} session(s) in cwd")
        } else {
            "no saved sessions in cwd".to_owned()
        };
        println!("  {slug:<12} {status:<15} ({session_info})");
    }
    println!("\nRun `mena agent <provider>` to launch an agent directly.");
}

pub fn run_sessions(args: &SessionsArgs, settings: &Settings) -> Result<()> {
    if args.limit == Some(0) {
        bail!("--limit must be at least 1");
    }
    let provider = args
        .provider
        .as_deref()
        .map(|provider| {
            AgentKind::from_slug(provider).with_context(|| {
                format!(
                    "unsupported session provider `{provider}`; use claude, codex, cursor, gemini, opencode, pi, or omp"
                )
            })
        })
        .transpose()?;
    let catalog = scan_sessions(provider.as_ref())?;
    let count = args
        .limit
        .unwrap_or_else(|| catalog.sessions().len())
        .min(catalog.sessions().len());
    let sessions = &catalog.sessions()[..count];
    if args.json {
        let values: Vec<_> = sessions.iter().map(session_list_json).collect();
        println!("{}", serde_json::to_string_pretty(&values)?);
    } else if io::stdin().is_terminal() && io::stdout().is_terminal() {
        let protection = session_protection(&catalog, settings)?;
        let export_directory =
            std::env::current_dir().context("failed to resolve the session export directory")?;
        let mut clipboard = crate::clipboard::SessionClipboard::default();
        let selected = tui::manage_sessions(
            sessions.to_vec(),
            protection,
            &settings.ui.session_detail.colors,
            |session| catalog.detail(session),
            |detail, scope| crate::export::export_session_detail(detail, &export_directory, scope),
            |detail, scope| clipboard.copy_detail(detail, scope),
            |session| {
                if session_protection(&catalog, settings)?
                    .protected_targets
                    .contains(&session.target())
                {
                    bail!("cannot delete a session that may be attached to a running agent");
                }
                catalog.delete_session(session)
            },
        )?;
        if let Some(session) = selected {
            resume_target(&session.target(), settings)?;
        }
    } else {
        print!("{}", render_session_table(sessions, None));
    }
    Ok(())
}
/// Execute `mena skills` operations.
///
/// # Errors
///
/// Returns an error if scanning fails or a targeted skill is missing.
pub fn run_skills(args: &SkillsArgs, _settings: &Settings) -> Result<()> {
    let home = dirs::home_dir();
    let current_dir = std::env::current_dir().ok();
    let catalog = SkillCatalog::scan(home.as_deref(), current_dir.as_deref())?;

    match &args.command {
        Some(SkillSubcommand::Inspect { name, json }) => {
            let detail = catalog.inspect(name, args.provider.as_deref(), args.scope.as_deref())?;
            if *json || args.json {
                println!("{}", serde_json::to_string_pretty(&detail)?);
            } else {
                print!("{}", render_skill_detail(&detail));
            }
        }
        None => {
            let filtered = catalog.filter(args.provider.as_deref(), args.scope.as_deref())?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&filtered)?);
            } else if filtered.is_empty() {
                ui::info("no skills discovered");
            } else if io::stdin().is_terminal() && io::stdout().is_terminal() {
                tui::manage_skills(filtered, |skill, path| catalog.entry(skill, path))?;
            } else {
                print!("{}", render_skill_table(&filtered));
            }
        }
    }

    Ok(())
}

fn resume_target(target: &str, settings: &Settings) -> Result<()> {
    let (provider, session_id) = split_session_selector(target, &settings.agent.custom);
    if let Some(name) = provider.filter(|name| settings.agent.custom.contains_key(*name)) {
        let custom = &settings.agent.custom[name];
        let spec = custom_resume_spec(name, custom, session_id)?;
        return execute_resume(&spec, &AgentKind::Custom(name.to_owned()), session_id, None);
    }
    let provider_kind = provider.and_then(AgentKind::from_slug);
    let catalog = scan_sessions(provider_kind.as_ref())?;
    let session = catalog.resolve(provider, session_id)?;
    let (kind, id, project) = (
        session.kind.clone(),
        session.id.clone(),
        session.project.clone(),
    );
    let spec = native_resume_command(&kind, &id)?;
    execute_resume(&spec, &kind, &id, project)
}

fn execute_resume(
    spec: &NativeResumeCommand,
    kind: &AgentKind,
    id: &str,
    project: Option<std::path::PathBuf>,
) -> Result<()> {
    let mut command = Command::new(&spec.program);
    command.args(&spec.args);
    if let Some(project) = project {
        if !project.is_dir() {
            bail!(
                "cannot resume {}:{} because its project directory no longer exists: {}",
                kind.slug(),
                id,
                project.display()
            );
        }
        command.current_dir(project);
    }
    ui::info(format!("resuming {}:{}", kind.slug(), id));
    let status = command.status().with_context(|| {
        format!(
            "failed to start `{}`; install it or ensure it is available on PATH",
            spec.program
        )
    })?;
    if !status.success() {
        bail!(
            "{} exited without successfully resuming session {} (status: {status})",
            spec.program,
            id
        );
    }
    Ok(())
}

fn custom_resume_spec(
    name: &str,
    settings: &CustomAgentSettings,
    id: &str,
) -> Result<NativeResumeCommand> {
    let (program, args) = settings
        .resume
        .split_first()
        .with_context(|| format!("custom agent `{name}` does not define a resume argv"))?;
    Ok(NativeResumeCommand {
        program: program.replace("{session}", id),
        args: args
            .iter()
            .map(|part| part.replace("{session}", id))
            .collect(),
    })
}

fn scan_sessions(provider: Option<&AgentKind>) -> Result<SessionCatalog> {
    let home =
        dirs::home_dir().context("could not resolve the home directory for agent sessions")?;
    SessionCatalog::scan_provider(&home, provider)
}

fn session_protection(
    catalog: &SessionCatalog,
    settings: &Settings,
) -> Result<crate::session::SessionProtection> {
    let live = discover_live_agents(&settings.agent.custom)?;
    catalog.protection(&live)
}

fn split_session_selector<'a>(
    target: &'a str,
    custom: &std::collections::BTreeMap<String, CustomAgentSettings>,
) -> (Option<&'a str>, &'a str) {
    target
        .split_once(':')
        .filter(|(name, _)| is_provider(name, custom))
        .map_or((None, target), |(name, value)| (Some(name), value))
}

fn is_provider(
    name: &str,
    custom: &std::collections::BTreeMap<String, CustomAgentSettings>,
) -> bool {
    matches!(
        name,
        "claude" | "codex" | "gemini" | "opencode" | "pi" | "omp" | "cursor"
    ) || custom.contains_key(name)
}

fn session_list_json(session: &AgentSession) -> Value {
    serde_json::json!({
        "target": session.target(),
        "agent": session.kind.slug(),
        "session_id": session.id,
        "title": session.title,
        "project": session.project,
        "log": session.path,
        "started_at": session.started_at,
        "updated_at_unix": session.updated_at,
    })
}

fn process_list_json(agent: &LiveAgent, verbose: bool) -> Value {
    let mut value = serde_json::json!({
        "kind": agent.kind.slug(),
        "pid": agent.process.pid,
        "cwd": agent.process.cwd,
        "run_time_seconds": agent.process.run_time,
        "status": agent.process.status,
    });
    if verbose {
        value["command"] = Value::String(agent.process.command.join(" "));
    }
    value
}
