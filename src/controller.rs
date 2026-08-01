use std::collections::BTreeSet;
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::process::{
    AgentKind, LiveAgent, discover_live_agents, discover_live_agents_with_cpu, stop_agent,
};
use crate::session::{AgentSession, SessionCatalog, UsageCache, tail_records};
use crate::settings::{CustomAgentSettings, Settings};
use crate::tui;
use crate::ui;
use crate::view::{
    AgentReport, format_bytes, format_duration, render_process_table, render_session_table,
};
use crate::{LogsArgs, PsArgs, ResumeArgs, SessionsArgs, StopArgs, TargetArgs, TopArgs};

pub fn run_ps(args: &PsArgs, settings: &Settings) -> Result<()> {
    let agents = discover_live_agents(&settings.agent.custom)?;
    let mut usage_cache = UsageCache::default();
    let reports = agent_reports(agents, &mut usage_cache)?;
    if args.json {
        let rows: Vec<_> = reports.iter().map(process_json).collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else {
        print!("{}", render_process_table(&reports, false, None));
    }
    Ok(())
}

pub fn run_top(args: &TopArgs, settings: &Settings) -> Result<()> {
    if args.interval == 0 {
        bail!("--interval must be at least 1 second");
    }
    if args.iterations == Some(0) {
        bail!("--iterations must be at least 1");
    }
    if args.iterations.is_none() && io::stdout().is_terminal() && io::stdin().is_terminal() {
        let mut usage_cache = UsageCache::default();
        return tui::run_top(Duration::from_secs(args.interval), || {
            let agents = discover_live_agents_with_cpu(&settings.agent.custom, true)?;
            agent_reports(agents, &mut usage_cache)
        });
    }

    let iterations = args.iterations.unwrap_or(1);
    let mut usage_cache = UsageCache::default();
    for iteration in 0..iterations {
        let agents = discover_live_agents_with_cpu(&settings.agent.custom, true)?;
        let reports = agent_reports(agents, &mut usage_cache)?;
        println!("mena top — {} running\n", reports.len());
        print!("{}", render_process_table(&reports, true, None));
        io::stdout()
            .flush()
            .context("failed to refresh agent view")?;
        if iteration + 1 < iterations {
            thread::sleep(Duration::from_secs(args.interval));
        }
    }
    Ok(())
}

pub fn run_inspect(args: &TargetArgs, settings: &Settings) -> Result<()> {
    if is_live_selector(&args.target, &settings.agent.custom) {
        let live = discover_live_agents(&settings.agent.custom)?;
        let agent = resolve_live(&live, &args.target, &settings.agent.custom)
            .with_context(|| format!("running agent not found: {}", args.target))?;
        let catalog = scan_sessions(Some(&agent.kind))?;
        let session = catalog
            .latest_for_process(
                &agent.kind,
                agent.process.cwd.as_deref(),
                agent.process.started_at,
            )
            .map(|session| catalog.with_usage(session))
            .transpose()?;
        let report = AgentReport {
            agent: agent.clone(),
            session: session.clone(),
        };
        if args.json {
            let mut value = process_json(&report);
            value["session"] = session.as_ref().map_or(Value::Null, session_json);
            println!("{}", serde_json::to_string_pretty(&value)?);
        } else {
            print_live_details(agent, session.as_ref());
        }
        return Ok(());
    }

    let (provider, session_id) = split_session_selector(&args.target, &settings.agent.custom);
    if provider.is_some_and(|name| settings.agent.custom.contains_key(name)) {
        bail!(
            "custom agent sessions do not define a local log catalog; inspect a live PID instead"
        );
    }
    let provider_kind = provider.and_then(AgentKind::from_slug);
    let catalog = scan_sessions(provider_kind.as_ref())?;
    let session = catalog.with_usage(catalog.resolve(provider, session_id)?)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&session_json(&session))?);
    } else {
        print_session_details(&session);
    }
    Ok(())
}

pub fn run_logs(args: &LogsArgs, settings: &Settings) -> Result<()> {
    let catalog;
    let session = if is_live_selector(&args.target, &settings.agent.custom) {
        let live = discover_live_agents(&settings.agent.custom)?;
        let agent = resolve_live(&live, &args.target, &settings.agent.custom)
            .with_context(|| format!("running agent not found: {}", args.target))?;
        catalog = scan_sessions(Some(&agent.kind))?;
        catalog
            .latest_for_process(
                &agent.kind,
                agent.process.cwd.as_deref(),
                agent.process.started_at,
            )
            .with_context(|| {
                format!(
                    "no local {} session could be matched to {}; pass a session ID instead",
                    agent.kind, args.target
                )
            })?
    } else {
        let (provider, session_id) = split_session_selector(&args.target, &settings.agent.custom);
        if provider.is_some_and(|name| settings.agent.custom.contains_key(name)) {
            bail!(
                "custom agent `{}` does not define a local session log catalog",
                provider.unwrap_or_default()
            );
        }
        let provider_kind = provider.and_then(AgentKind::from_slug);
        catalog = scan_sessions(provider_kind.as_ref())?;
        catalog.resolve(provider, session_id)?
    };

    for record in tail_records(&session.path, args.lines)? {
        if args.raw {
            println!("{record}");
        } else {
            println!("{}", summarize_record(&record));
        }
    }
    Ok(())
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
                    "unsupported session provider `{provider}`; use claude, codex, gemini, opencode, pi, or omp"
                )
            })
        })
        .transpose()?;
    if provider == Some(AgentKind::Cursor) {
        bail!("Cursor does not expose a supported local session catalog");
    }
    let catalog = scan_sessions(provider.as_ref())?;
    let count = args
        .limit
        .unwrap_or_else(|| catalog.sessions().len())
        .min(catalog.sessions().len());
    let sessions = &catalog.sessions()[..count];
    if args.json {
        let values: Vec<_> = sessions.iter().map(session_list_json).collect();
        println!("{}", serde_json::to_string_pretty(&values)?);
    } else if !args.plain && io::stdin().is_terminal() && io::stdout().is_terminal() {
        let active_targets = active_session_targets(&catalog, settings)?;
        let export_directory =
            std::env::current_dir().context("failed to resolve the session export directory")?;
        let mut clipboard = crate::clipboard::SessionClipboard::default();
        let selected = tui::manage_sessions(
            sessions.to_vec(),
            active_targets,
            |session| catalog.detail(session),
            |detail| crate::export::export_session_detail(detail, &export_directory),
            |detail| clipboard.copy_detail(detail),
            |session| {
                if active_session_targets(&catalog, settings)?.contains(&session_target(session)) {
                    bail!("cannot delete a session that is attached to a running agent");
                }
                catalog.delete_session(session)
            },
        )?;
        if let Some(session) = selected {
            resume_target(&session_target(&session), settings)?;
        }
    } else {
        print!("{}", render_session_table(sessions, None));
    }
    Ok(())
}

pub fn run_stop(args: &StopArgs, settings: &Settings) -> Result<()> {
    if !is_live_selector(&args.target, &settings.agent.custom) {
        bail!("mena stop requires a PID or provider:PID selector");
    }
    let live = discover_live_agents(&settings.agent.custom)?;
    let agent = resolve_live(&live, &args.target, &settings.agent.custom)
        .with_context(|| format!("running agent not found: {}", args.target))?;
    stop_agent(agent, args.force, &settings.agent.custom)?;
    let signal = if args.force {
        "force-stop"
    } else {
        "termination"
    };
    ui::success(format!(
        "sent {signal} signal to {}:{}",
        agent.kind.slug(),
        agent.process.pid
    ));
    Ok(())
}

pub fn run_resume(args: &ResumeArgs, settings: &Settings) -> Result<()> {
    if args.limit == 0 {
        bail!("--limit must be at least 1");
    }
    if args.list {
        let catalog = scan_sessions(None)?;
        let sessions = &catalog.sessions()[..catalog.sessions().len().min(args.limit)];
        print!("{}", render_session_table(sessions, None));
        return Ok(());
    }

    let target = if let Some(target) = &args.target {
        target.clone()
    } else {
        let catalog = scan_sessions(None)?;
        let sessions = &catalog.sessions()[..catalog.sessions().len().min(args.limit)];
        let selected = if args.last {
            sessions.first().cloned()
        } else {
            if sessions.is_empty() {
                bail!("no saved developer-agent sessions were found");
            }
            if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
                bail!(
                    "interactive session selection requires a terminal; use `mena resume --list`"
                );
            }
            tui::pick_session(sessions.to_vec())?
        };
        let Some(session) = selected else {
            return Ok(());
        };
        format!("{}:{}", session.kind.slug(), session.id)
    };
    resume_target(&target, settings)
}

fn resume_target(target: &str, settings: &Settings) -> Result<()> {
    let (provider, session_id) = split_session_selector(target, &settings.agent.custom);
    if let Some(name) = provider.filter(|name| settings.agent.custom.contains_key(*name)) {
        let custom = &settings.agent.custom[name];
        let spec = custom_resume_spec(name, custom, session_id)?;
        return execute_resume(&spec, &AgentKind::Custom(name.to_owned()), session_id, None);
    }
    let provider_kind = provider.and_then(AgentKind::from_slug);
    let (kind, id, project) = if provider_kind == Some(AgentKind::Cursor) {
        (AgentKind::Cursor, session_id.to_owned(), None)
    } else {
        let catalog = scan_sessions(provider_kind.as_ref())?;
        let session = catalog.resolve(provider, session_id)?;
        (
            session.kind.clone(),
            session.id.clone(),
            session.project.clone(),
        )
    };
    let spec = resume_spec(&kind, &id)?;
    execute_resume(&spec, &kind, &id, project)
}

fn execute_resume(
    spec: &ResumeSpec,
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

#[derive(Debug, PartialEq, Eq)]
struct ResumeSpec {
    program: String,
    args: Vec<String>,
}

fn resume_spec(kind: &AgentKind, id: &str) -> Result<ResumeSpec> {
    let (program, args): (&str, Vec<String>) = match kind {
        AgentKind::ClaudeCode => ("claude", vec!["--resume".to_owned(), id.to_owned()]),
        AgentKind::Codex => ("codex", vec!["resume".to_owned(), id.to_owned()]),
        AgentKind::GeminiCli => ("gemini", vec!["--resume".to_owned(), id.to_owned()]),
        AgentKind::OpenCode => ("opencode", vec!["--session".to_owned(), id.to_owned()]),
        AgentKind::Pi => ("pi", vec!["--session".to_owned(), id.to_owned()]),
        AgentKind::OhMyPi => ("omp", vec!["--resume".to_owned(), id.to_owned()]),
        AgentKind::Cursor => ("cursor-agent", vec!["--resume".to_owned(), id.to_owned()]),
        AgentKind::Custom(name) => {
            bail!("custom agent {name} does not define a resume command")
        }
    };
    Ok(ResumeSpec {
        program: program.to_owned(),
        args,
    })
}

fn custom_resume_spec(name: &str, settings: &CustomAgentSettings, id: &str) -> Result<ResumeSpec> {
    let (program, args) = settings
        .resume
        .split_first()
        .with_context(|| format!("custom agent `{name}` does not define a resume argv"))?;
    Ok(ResumeSpec {
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

fn agent_reports(agents: Vec<LiveAgent>, usage_cache: &mut UsageCache) -> Result<Vec<AgentReport>> {
    let catalog = scan_sessions(None)?;
    agents
        .into_iter()
        .map(|agent| {
            let session = catalog
                .latest_for_process(
                    &agent.kind,
                    agent.process.cwd.as_deref(),
                    agent.process.started_at,
                )
                .map(|session| usage_cache.enrich(&catalog, session))
                .transpose()?;
            Ok(AgentReport { agent, session })
        })
        .collect()
}

fn active_session_targets(
    catalog: &SessionCatalog,
    settings: &Settings,
) -> Result<BTreeSet<String>> {
    let live = discover_live_agents(&settings.agent.custom)?;
    Ok(live
        .iter()
        .filter_map(|agent| {
            catalog
                .latest_for_process(
                    &agent.kind,
                    agent.process.cwd.as_deref(),
                    agent.process.started_at,
                )
                .map(session_target)
        })
        .collect())
}

fn session_target(session: &AgentSession) -> String {
    format!("{}:{}", session.kind.slug(), session.id)
}

fn resolve_live<'a>(
    agents: &'a [LiveAgent],
    target: &str,
    custom: &std::collections::BTreeMap<String, CustomAgentSettings>,
) -> Option<&'a LiveAgent> {
    let (provider, remainder) = target
        .split_once(':')
        .filter(|(name, _)| is_provider(name, custom))
        .map_or((None, target), |(name, value)| (Some(name), value));
    let Ok(pid) = remainder.parse::<u32>() else {
        return None;
    };
    agents.iter().find(|agent| {
        agent.process.pid == pid && provider.is_none_or(|name| agent.kind.slug() == name)
    })
}

fn is_live_selector(
    target: &str,
    custom: &std::collections::BTreeMap<String, CustomAgentSettings>,
) -> bool {
    let remainder = target
        .split_once(':')
        .filter(|(name, _)| is_provider(name, custom))
        .map_or(target, |(_, value)| value);
    remainder.parse::<u32>().is_ok()
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

fn process_json(report: &AgentReport) -> Value {
    let agent = &report.agent;
    serde_json::json!({
        "id": format!("{}:{}", agent.kind.slug(), agent.process.pid),
        "agent": agent.kind.slug(),
        "pid": agent.process.pid,
        "parent_pid": agent.process.parent_pid,
        "executable": agent.process.executable,
        "project": report.project(),
        "status": agent.process.status,
        "started_at_unix": agent.process.started_at,
        "duration_seconds": agent.process.run_time,
        "cpu_percent": agent.process.cpu_percent,
        "memory_bytes": agent.process.memory_bytes,
        "session_id": report.session.as_ref().map(|session| &session.id),
        "tokens": report.session.as_ref().and_then(|session| session.tokens),
        "cost_usd": report.session.as_ref().and_then(|session| session.cost_usd),
        "cost_status": report.session.as_ref().map(|session| if session.cost_usd.is_some() { "recorded" } else { "not_recorded" }),
    })
}

fn session_json(session: &AgentSession) -> Value {
    serde_json::json!({
        "agent": session.kind.slug(),
        "session_id": session.id,
        "title": session.title,
        "project": session.project,
        "log": session.path,
        "started_at": session.started_at,
        "updated_at_unix": session.updated_at,
        "tokens": session.tokens,
        "cost_usd": session.cost_usd,
    })
}

fn session_list_json(session: &AgentSession) -> Value {
    serde_json::json!({
        "target": format!("{}:{}", session.kind.slug(), session.id),
        "agent": session.kind.slug(),
        "session_id": session.id,
        "title": session.title,
        "project": session.project,
        "log": session.path,
        "started_at": session.started_at,
        "updated_at_unix": session.updated_at,
    })
}

fn print_live_details(agent: &LiveAgent, session: Option<&AgentSession>) {
    println!("ID:          {}:{}", agent.kind.slug(), agent.process.pid);
    println!("Agent:       {}", agent.kind);
    println!("PID:         {}", agent.process.pid);
    println!(
        "Parent PID:  {}",
        agent
            .process
            .parent_pid
            .map_or_else(|| "-".to_owned(), |pid| pid.to_string())
    );
    println!("Executable:  {}", agent.process.executable.display());
    println!(
        "Project:     {}",
        display_path(
            session
                .and_then(|session| session.project.as_deref())
                .or(agent.process.cwd.as_deref())
        )
    );
    println!("Status:      {}", agent.process.status);
    println!("Duration:    {}", format_duration(agent.process.run_time));
    println!("CPU:         {:.1}%", agent.process.cpu_percent);
    println!("Memory:      {}", format_bytes(agent.process.memory_bytes));
    println!("Command:     {}", redacted_command(&agent.process.command));
    if let Some(session) = session {
        println!();
        println!("Matched session (latest for this agent and project):");
        print_session_details(session);
    } else {
        println!("Session:     -");
        println!("Tokens:      -");
        println!("Cost:        -");
    }
}

fn print_session_details(session: &AgentSession) {
    println!("Session:     {}:{}", session.kind.slug(), session.id);
    println!("Agent:       {}", session.kind);
    println!("Title:       {}", session.title.as_deref().unwrap_or("-"));
    println!("Project:     {}", display_path(session.project.as_deref()));
    println!(
        "Started:     {}",
        session.started_at.as_deref().unwrap_or("-")
    );
    println!("Updated:     {}", format_unix_timestamp(session.updated_at));
    println!("Tokens:      {}", format_tokens(session.tokens));
    println!("Cost:        {}", format_cost(session.cost_usd));
    println!("Log:         {}", session.path.display());
}

fn redacted_command(command: &[String]) -> String {
    let mut redacted = Vec::with_capacity(command.len());
    let mut hide_next = false;
    for part in command {
        if hide_next {
            redacted.push("[REDACTED]".to_owned());
            hide_next = false;
            continue;
        }
        let lower = part.to_ascii_lowercase();
        let sensitive = [
            "token", "secret", "password", "api-key", "api_key", "cookie",
        ]
        .iter()
        .any(|marker| lower.contains(marker));
        if sensitive {
            if let Some((name, _)) = part.split_once('=') {
                redacted.push(format!("{name}=[REDACTED]"));
            } else {
                redacted.push(part.clone());
                hide_next = true;
            }
        } else {
            redacted.push(part.clone());
        }
    }
    truncate(&redacted.join(" "), 1_000)
}

fn summarize_record(record: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(record) else {
        return truncate(record, 500);
    };
    let timestamp = ["/timestamp", "/payload/timestamp", "/message/timestamp"]
        .iter()
        .find_map(|pointer| value.pointer(pointer).and_then(Value::as_str))
        .unwrap_or("-");
    let kind = value
        .pointer("/payload/type")
        .or_else(|| value.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("event");
    let role = value
        .pointer("/message/role")
        .or_else(|| value.pointer("/payload/role"))
        .or_else(|| value.get("role"))
        .and_then(Value::as_str);
    let content = ["/message/content", "/payload/content", "/content"]
        .iter()
        .find_map(|pointer| value.pointer(pointer))
        .and_then(content_text);
    let label = role.map_or_else(|| kind.to_owned(), |role| format!("{kind}/{role}"));
    content.map_or_else(
        || format!("{timestamp}  {label}"),
        |content| {
            format!(
                "{timestamp}  {label}  {}",
                truncate(&one_line(&content), 500)
            )
        },
    )
}

fn content_text(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_owned());
    }
    let parts: Vec<&str> = value
        .as_array()?
        .iter()
        .filter_map(|part| {
            part.get("text")
                .or_else(|| part.get("content"))
                .and_then(Value::as_str)
        })
        .collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

fn display_path(path: Option<&Path>) -> String {
    path.map_or_else(|| "-".to_owned(), |path| path.display().to_string())
}

fn format_tokens(tokens: Option<u64>) -> String {
    tokens.map_or_else(|| "-".to_owned(), |tokens| tokens.to_string())
}

fn format_cost(cost: Option<f64>) -> String {
    cost.map_or_else(|| "n/a".to_owned(), |cost| format!("${cost:.4}"))
}

fn format_unix_timestamp(timestamp: u64) -> String {
    i64::try_from(timestamp)
        .ok()
        .and_then(|timestamp| chrono::DateTime::from_timestamp(timestamp, 0))
        .map_or_else(|| timestamp.to_string(), |value| value.to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::{AgentKind, ResumeSpec, redacted_command, resume_spec, summarize_record};

    #[test]
    fn command_display_redacts_secret_argument_values() {
        assert_eq!(
            redacted_command(&[
                "agent".to_owned(),
                "--api-key".to_owned(),
                "secret-value".to_owned(),
                "--model=x".to_owned(),
                "--token=secret".to_owned(),
            ]),
            "agent --api-key [REDACTED] --model=x --token=[REDACTED]"
        );
    }

    #[test]
    fn log_summary_keeps_role_and_text_without_dumping_json() {
        let summary = summarize_record(
            r#"{"type":"message","timestamp":"now","message":{"role":"assistant","content":[{"type":"text","text":"done\nnow"}],"private":"hidden"}}"#,
        );
        assert_eq!(summary, "now  message/assistant  done now");
        assert!(!summary.contains("private"));
    }

    #[test]
    fn resume_commands_use_native_argv_without_a_shell() {
        let cases = [
            (
                AgentKind::ClaudeCode,
                ResumeSpec {
                    program: "claude".to_owned(),
                    args: vec!["--resume".to_owned(), "session-id".to_owned()],
                },
            ),
            (
                AgentKind::Codex,
                ResumeSpec {
                    program: "codex".to_owned(),
                    args: vec!["resume".to_owned(), "session-id".to_owned()],
                },
            ),
            (
                AgentKind::GeminiCli,
                ResumeSpec {
                    program: "gemini".to_owned(),
                    args: vec!["--resume".to_owned(), "session-id".to_owned()],
                },
            ),
            (
                AgentKind::OpenCode,
                ResumeSpec {
                    program: "opencode".to_owned(),
                    args: vec!["--session".to_owned(), "session-id".to_owned()],
                },
            ),
            (
                AgentKind::Pi,
                ResumeSpec {
                    program: "pi".to_owned(),
                    args: vec!["--session".to_owned(), "session-id".to_owned()],
                },
            ),
            (
                AgentKind::OhMyPi,
                ResumeSpec {
                    program: "omp".to_owned(),
                    args: vec!["--resume".to_owned(), "session-id".to_owned()],
                },
            ),
            (
                AgentKind::Cursor,
                ResumeSpec {
                    program: "cursor-agent".to_owned(),
                    args: vec!["--resume".to_owned(), "session-id".to_owned()],
                },
            ),
        ];
        for (kind, expected) in cases {
            assert_eq!(
                resume_spec(&kind, "session-id").expect("resume spec"),
                expected
            );
        }
    }
}
