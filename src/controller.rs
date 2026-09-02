use std::io::{self, IsTerminal, Write};
use std::process::Command;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::continuation::{continuation_targets, prepare_continuation};
use crate::mcp::{McpCatalog, McpProbeStatus, McpRegistration};
use crate::memory::MemoryCatalog;
use crate::process::{AgentKind, LiveAgent, discover_live_agents};
use crate::session::{
    AgentSession, NativeResumeCommand, SessionCatalog, native_resume_command,
    session_provider_slugs,
};
use crate::settings::{CustomAgentSettings, Settings};
use crate::skill::SkillCatalog;
use crate::tui;
pub use crate::ui;
use crate::view::{
    render_mcp_detail, render_mcp_table, render_memory_detail, render_memory_table,
    render_process_table, render_session_table, render_skill_detail, render_skill_table,
};
use crate::{
    AgentLaunchArgs, McpArgs, McpSubcommand, MemoriesArgs, MemorySubcommand, PsArgs,
    SessionSubcommand, SessionsArgs, SkillSubcommand, SkillsArgs,
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
            let line = catalog.source_line(&detail.registration)?;
            crate::editor::open_file_at_line(&detail.registration.source, line)?;
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
                run_mcp_tui(&catalog, args, registrations.into_iter().cloned().collect())?;
            } else {
                print!("{}", render_mcp_table(&registrations));
            }
        }
    }
    Ok(())
}

fn run_mcp_tui(
    catalog: &Arc<McpCatalog>,
    args: &McpArgs,
    registrations: Vec<McpRegistration>,
) -> Result<()> {
    let probe_catalog = Arc::clone(catalog);
    let locate_catalog = Arc::clone(catalog);
    let refresh_catalog = Arc::clone(catalog);
    let delete_catalog = Arc::clone(catalog);
    let provider = args.provider.clone();
    let scope = args.scope.clone();
    let source = args.source.clone();
    tui::manage_mcp(
        registrations,
        move |registration| probe_catalog.inspect_current_registration_with_probe(registration, 10),
        move |registration| locate_catalog.source_line(registration),
        move || {
            refresh_catalog.refresh_selection(
                provider.as_deref(),
                scope.as_deref(),
                source.as_deref(),
            )
        },
        move |registration| delete_catalog.delete_registration(registration),
    )
}

/// Execute `mena memories` with purely static discovery and bounded reads.
///
/// # Errors
///
/// Returns an actionable error when a selector is ambiguous, a file cannot be
/// read within the size bound, or a deletion is refused by validation.
pub fn run_memories(args: &MemoriesArgs) -> Result<()> {
    let home = dirs::home_dir();
    let current_dir = std::env::current_dir().context("could not resolve current directory")?;
    let catalog = MemoryCatalog::scan(home.as_deref(), Some(&current_dir))?;

    match &args.command {
        Some(MemorySubcommand::Inspect { name, json }) => {
            let detail = catalog.inspect(name, args.provider.as_deref(), args.scope.as_deref())?;
            if *json || args.json {
                println!("{}", serde_json::to_string_pretty(&detail)?);
            } else {
                print!("{}", render_memory_detail(&detail));
            }
        }
        Some(MemorySubcommand::Open { name }) => {
            if args.json {
                bail!("--json cannot be used with `mena memories open`");
            }
            let file = catalog.resolve(name, args.provider.as_deref(), args.scope.as_deref())?;
            crate::editor::edit_file_at_line(&file.path, 1)?;
            ui::success(format!("edited memory file {}", file.path.display()));
        }
        Some(MemorySubcommand::Delete { name }) => {
            if args.json {
                bail!("--json cannot be used with `mena memories delete`");
            }
            let file = catalog.resolve(name, args.provider.as_deref(), args.scope.as_deref())?;
            confirm_delete_memory(&file.path)?;
            let removed = catalog.delete(&file.path)?;
            ui::success(format!("deleted memory file {}", removed.display()));
        }
        None => {
            let filtered = catalog.filter(args.provider.as_deref(), args.scope.as_deref())?;
            if args.json {
                println!("{}", serde_json::to_string_pretty(&filtered)?);
            } else if filtered.is_empty() {
                ui::info("no memory files discovered");
            } else {
                print!("{}", render_memory_table(&filtered));
            }
        }
    }

    Ok(())
}

fn confirm_delete_memory(path: &std::path::Path) -> Result<()> {
    print!("delete memory file {}? [y/N] ", path.display());
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if answer.trim() != "y" {
        bail!("aborted; memory file was not deleted");
    }
    Ok(())
}

pub fn run_agent(args: &AgentLaunchArgs, settings: &Settings) -> Result<()> {
    let cwd = std::env::current_dir().context("could not resolve current working directory")?;
    let catalog = scan_sessions(None, false)?;
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
            "unsupported agent provider `{slug}`; available providers: claude, codex, goose, grok, omp, opencode, pi, cursor, gemini{}",
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
        AgentKind::Grok => Ok(NativeResumeCommand {
            program: "grok".to_owned(),
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
    // Deletion re-checks live-session protection before each removal, but
    // a confirmed batch issues those checks back-to-back. Sharing one
    // snapshot within a short window keeps a batch from re-running
    // process discovery per session while still refreshing between
    // separate user actions.
    const PROTECTION_SNAPSHOT_WINDOW: std::time::Duration = std::time::Duration::from_millis(250);
    match &args.command {
        Some(SessionSubcommand::Rename { target, title }) => {
            return rename_session(args, settings, target, title);
        }
        None => {}
    }
    let provider = args
        .provider
        .as_deref()
        .map(|provider| {
            AgentKind::from_slug(provider).with_context(|| {
                format!(
                    "unsupported session provider `{provider}`; use {}",
                    session_provider_slugs().join(", ")
                )
            })
        })
        .transpose()?;
    let catalog = scan_sessions(provider.as_ref(), args.include_empty)?;
    let count = args
        .limit
        .and_then(|limit| usize::try_from(limit).ok())
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
        let protection_cache = std::cell::RefCell::new(
            None::<(std::time::Instant, crate::session::SessionProtection)>,
        );
        let selected = tui::manage_sessions(
            sessions.to_vec(),
            protection,
            &settings.ui.session_detail.colors,
            |session| catalog.detail(session),
            |detail, scope| crate::export::export_session_detail(detail, &export_directory, scope),
            |detail, scope| clipboard.copy_detail(detail, scope),
            |session| {
                let protected = {
                    let mut cache = protection_cache.borrow_mut();
                    let fresh = cache.as_ref().is_some_and(|(taken_at, _)| {
                        taken_at.elapsed() < PROTECTION_SNAPSHOT_WINDOW
                    });
                    if !fresh {
                        *cache = Some((
                            std::time::Instant::now(),
                            session_protection(&catalog, settings)?,
                        ));
                    }
                    cache
                        .as_ref()
                        .expect("protection snapshot was just stored")
                        .1
                        .protected_targets
                        .contains(&session.target())
                };
                if protected {
                    bail!("cannot delete a session that may be attached to a running agent");
                }
                catalog.delete_session(session)
            },
            |session, title| catalog.set_title(session, title),
        )?;
        if let Some(action) = selected {
            match action {
                tui::session::SessionBrowserResult::Resume(session) => {
                    resume_target(&session.target(), settings)?;
                }
                tui::session::SessionBrowserResult::ContinueWith(session) => {
                    continue_with_agent(&catalog, &session, settings)?;
                }
            }
        }
    } else {
        print!("{}", render_session_table(sessions));
    }
    Ok(())
}

fn rename_session(
    args: &SessionsArgs,
    settings: &Settings,
    target: &str,
    title: &str,
) -> Result<()> {
    if args.json {
        bail!("--json cannot be used with `mena ss rename`");
    }
    let provider = args
        .provider
        .as_deref()
        .map(|provider| {
            AgentKind::from_slug(provider).with_context(|| {
                format!(
                    "unsupported session provider `{provider}`; use {}",
                    session_provider_slugs().join(", ")
                )
            })
        })
        .transpose()?;
    let catalog = scan_sessions(provider.as_ref(), args.include_empty)?;
    let (selector_provider, id) = split_session_selector(target, &settings.agent.custom);
    let session = catalog
        .resolve(
            provider.as_ref().map(AgentKind::slug).or(selector_provider),
            id,
        )?
        .clone();
    match catalog.set_title(&session, title)? {
        Some(title) => ui::success(format!("renamed {} to {title}", session.target())),
        None => ui::success(format!(
            "restored the native title for {}",
            session.target()
        )),
    }
    Ok(())
}

fn continue_with_agent(
    catalog: &SessionCatalog,
    session: &AgentSession,
    settings: &Settings,
) -> Result<()> {
    let targets: Vec<_> = continuation_targets(&session.kind)
        .into_iter()
        .filter(|target| target.kind.is_installed(&settings.agent.custom))
        .collect();
    if targets.is_empty() {
        bail!("no supported continuation target is installed; install claude, codex, or omp");
    }
    let Some(target) = tui::select_continuation_target(session, &targets)? else {
        return Ok(());
    };
    let prepared = prepare_continuation(session, &target, || catalog.detail(session))?;
    if target.kind == AgentKind::OhMyPi {
        ui::info(format!(
            "OMP will open its importer; select {}",
            session.target()
        ));
    }
    if let Some(path) = prepared.handoff_path() {
        ui::info(format!(
            "created temporary private handoff {}",
            path.display()
        ));
    }
    execute_continuation(prepared.command(), session, &target.kind)
}

fn execute_continuation(
    spec: &NativeResumeCommand,
    source: &AgentSession,
    target: &AgentKind,
) -> Result<()> {
    let mut command = Command::new(&spec.program);
    command.args(&spec.args);
    if let Some(project) = source.project.as_deref() {
        if !project.is_dir() {
            bail!(
                "cannot continue {} with {} because its project directory no longer exists: {}",
                source.target(),
                target.slug(),
                project.display()
            );
        }
        command.current_dir(project);
    }
    ui::info(format!(
        "continuing {} with {}",
        source.target(),
        target.slug()
    ));
    let status = command.status().with_context(|| {
        format!(
            "failed to start `{}`; install it or ensure it is available on PATH",
            spec.program
        )
    })?;
    if !status.success() {
        bail!(
            "{} exited without continuing {} (status: {status})",
            spec.program,
            source.target()
        );
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
    let catalog = scan_sessions(provider_kind.as_ref(), false)?;
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

fn scan_sessions(provider: Option<&AgentKind>, include_empty: bool) -> Result<SessionCatalog> {
    let home =
        dirs::home_dir().context("could not resolve the home directory for agent sessions")?;
    SessionCatalog::scan_provider(&home, provider, include_empty)
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
    AgentKind::from_slug(name).is_some() || custom.contains_key(name)
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

#[cfg(test)]
mod tests {
    use super::split_session_selector;
    use std::collections::BTreeMap;

    #[test]
    fn grok_resume_targets_split_as_a_known_provider() {
        let custom = BTreeMap::new();
        let (provider, id) =
            split_session_selector("grok:01a05b12-60ae-7790-8716-9293782180a9", &custom);
        assert_eq!(provider, Some("grok"));
        assert_eq!(id, "01a05b12-60ae-7790-8716-9293782180a9");
    }
}
