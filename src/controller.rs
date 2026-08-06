use std::collections::BTreeSet;
use std::io::{self, IsTerminal};
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::SessionsArgs;
use crate::process::{AgentKind, discover_live_agents};
use crate::session::{AgentSession, NativeResumeCommand, SessionCatalog, native_resume_command};
use crate::settings::{CustomAgentSettings, Settings};
use crate::tui;
use crate::ui;
use crate::view::render_session_table;

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
    } else if io::stdin().is_terminal() && io::stdout().is_terminal() {
        let active_targets = active_session_targets(&catalog, settings)?;
        let export_directory =
            std::env::current_dir().context("failed to resolve the session export directory")?;
        let mut clipboard = crate::clipboard::SessionClipboard::default();
        let selected = tui::manage_sessions(
            sessions.to_vec(),
            active_targets,
            &settings.ui.session_detail.colors,
            |session| catalog.detail(session),
            |detail, scope| crate::export::export_session_detail(detail, &export_directory, scope),
            |detail, scope| clipboard.copy_detail(detail, scope),
            |session| {
                if active_session_targets(&catalog, settings)?.contains(&session.target()) {
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

fn active_session_targets(
    catalog: &SessionCatalog,
    settings: &Settings,
) -> Result<BTreeSet<String>> {
    let live = discover_live_agents(&settings.agent.custom)?;
    Ok(catalog
        .associate_processes(&live)?
        .protected_targets()
        .clone())
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
#[allow(dead_code)]
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

#[allow(dead_code)]
fn one_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[allow(dead_code)]
fn truncate(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

#[allow(dead_code)]
fn display_path(path: Option<&Path>) -> String {
    path.map_or_else(|| "-".to_owned(), |path| path.display().to_string())
}

#[allow(dead_code)]
fn format_tokens(tokens: Option<u64>) -> String {
    tokens.map_or_else(|| "-".to_owned(), |tokens| tokens.to_string())
}

#[allow(dead_code)]
fn format_cost(cost: Option<f64>) -> String {
    cost.map_or_else(|| "n/a".to_owned(), |cost| format!("${cost:.4}"))
}

#[allow(dead_code)]
fn format_unix_timestamp(timestamp: u64) -> String {
    i64::try_from(timestamp)
        .ok()
        .and_then(|timestamp| chrono::DateTime::from_timestamp(timestamp, 0))
        .map_or_else(|| timestamp.to_string(), |value| value.to_rfc3339())
}
