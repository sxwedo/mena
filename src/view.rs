use std::fmt::Write as _;

use unicode_width::UnicodeWidthStr;

use crate::mcp::{McpDetail, McpProbe, McpRegistration, McpServerCapabilities};
use crate::process::LiveAgent;
use crate::session::{
    AgentSession, MetricError, ModelUsageSummary, ResponseMetrics, TokenUsage, ToolMetrics,
};
use crate::skill::{AgentSkill, SkillDetail};

pub fn render_process_table(agents: &[LiveAgent], verbose: bool) -> String {
    if agents.is_empty() {
        return "No running developer-agent processes found.\n".to_owned();
    }

    let mut headers = vec!["KIND", "PID", "CWD", "RUNTIME", "STATUS"];
    if verbose {
        headers.push("COMMAND");
    }
    let rows = agents
        .iter()
        .map(|agent| {
            let mut row = vec![
                agent.kind.slug().to_owned(),
                agent.process.pid.to_string(),
                agent
                    .process
                    .cwd
                    .as_deref()
                    .map_or_else(|| "-".to_owned(), |cwd| cwd.display().to_string()),
                format_duration(agent.process.run_time),
                agent.process.status.clone(),
            ];
            if verbose {
                row.push(agent.process.command.join(" "));
            }
            row
        })
        .collect::<Vec<_>>();
    strip_terminal_controls(&render_table(&headers, &rows))
}

pub fn render_session_table(sessions: &[AgentSession], selected: Option<usize>) -> String {
    if sessions.is_empty() {
        return "No saved developer-agent sessions found.\n".to_owned();
    }

    let headers = ["TARGET", "AGENT", "PROJECT", "TITLE / SUMMARY", "UPDATED"];
    let rows: Vec<Vec<String>> = sessions
        .iter()
        .enumerate()
        .map(|(index, session)| {
            let mut row = vec![
                session.target(),
                session.kind.to_string(),
                session
                    .project
                    .as_deref()
                    .map_or_else(|| "-".to_owned(), project_label),
                session
                    .title
                    .clone()
                    .unwrap_or_else(|| "(untitled)".to_owned()),
                format_age(session.updated_at),
            ];
            if let Some(selected) = selected {
                row.insert(
                    0,
                    if selected == index {
                        ">".to_owned()
                    } else {
                        " ".to_owned()
                    },
                );
            }
            row
        })
        .collect();
    if selected.is_some() {
        render_table(
            &[
                "",
                "TARGET",
                "AGENT",
                "PROJECT",
                "TITLE / SUMMARY",
                "UPDATED",
            ],
            &rows,
        )
    } else {
        render_table(&headers, &rows)
    }
}

pub fn render_skill_table(skills: &[AgentSkill]) -> String {
    let bold_cyan = anstyle::Style::new()
        .fg_color(Some(anstyle::Color::Ansi(anstyle::AnsiColor::Cyan)))
        .bold();
    let green =
        anstyle::Style::new().fg_color(Some(anstyle::Color::Ansi(anstyle::AnsiColor::Green)));
    let cyan = anstyle::Style::new().fg_color(Some(anstyle::Color::Ansi(anstyle::AnsiColor::Cyan)));
    let magenta =
        anstyle::Style::new().fg_color(Some(anstyle::Color::Ansi(anstyle::AnsiColor::Magenta)));
    let yellow =
        anstyle::Style::new().fg_color(Some(anstyle::Color::Ansi(anstyle::AnsiColor::Yellow)));
    let reset = anstyle::Style::new();

    let headers = [
        "NAME",
        "PROVIDER",
        "TYPE",
        "LOCATION",
        "SCOPE",
        "TRIGGERS",
        "DESCRIPTION",
    ];
    let rows: Vec<Vec<String>> = skills
        .iter()
        .map(|skill| {
            let triggers = if skill.triggers.is_empty() {
                "-".to_string()
            } else {
                skill.triggers.join(", ")
            };
            let description = skill
                .description
                .as_deref()
                .unwrap_or("-")
                .split('\n')
                .next()
                .unwrap_or("-")
                .to_string();

            let scope_styled = if skill.scope == "global" {
                format!("{cyan}[{}]{reset}", skill.scope)
            } else {
                format!("{green}[{}]{reset}", skill.scope)
            };

            let provider_styled = match skill.provider.as_str() {
                "claude" => format!("{magenta}[{}]{reset}", skill.provider),
                "codex" => format!("{yellow}[{}]{reset}", skill.provider),
                _ => format!("[{}]", skill.provider),
            };

            let type_styled = if skill.is_symlink {
                format!("{yellow}[symlink]{reset}")
            } else {
                "[file]".to_string()
            };

            vec![
                format!("{bold_cyan}{}{reset}", skill.name),
                provider_styled,
                type_styled,
                skill.location.clone(),
                scope_styled,
                triggers,
                description,
            ]
        })
        .collect();
    render_table(&headers, &rows)
}

pub fn render_skill_detail(detail: &SkillDetail) -> String {
    let bold_magenta = anstyle::Style::new()
        .fg_color(Some(anstyle::Color::Ansi(anstyle::AnsiColor::Magenta)))
        .bold();
    let bold_cyan = anstyle::Style::new()
        .fg_color(Some(anstyle::Color::Ansi(anstyle::AnsiColor::Cyan)))
        .bold();
    let green =
        anstyle::Style::new().fg_color(Some(anstyle::Color::Ansi(anstyle::AnsiColor::Green)));
    let red = anstyle::Style::new().fg_color(Some(anstyle::Color::Ansi(anstyle::AnsiColor::Red)));
    let dim = anstyle::Style::new().dimmed();
    let reset = anstyle::Style::new();

    let mut out = String::new();
    let s = &detail.skill;

    let _ = writeln!(
        out,
        "╭─────────────────────────────────────────────────────────────"
    );
    let _ = writeln!(out, "│ {bold_cyan}Skill Inspector:{reset} {}", s.name);
    let _ = writeln!(
        out,
        "├─────────────────────────────────────────────────────────────"
    );
    let _ = writeln!(out, "│ {bold_magenta}Provider:{reset}    {}", s.provider);
    let _ = writeln!(out, "│ {bold_magenta}Scope:{reset}       {}", s.scope);
    let _ = writeln!(
        out,
        "│ {bold_magenta}Path:{reset}        {dim}{}{reset}",
        s.path.display()
    );
    let _ = writeln!(
        out,
        "│ {bold_magenta}Valid:{reset}       {}",
        if s.valid {
            format!("{green}✓ true{reset}")
        } else {
            format!("{red}✗ false{reset}")
        }
    );
    if !s.triggers.is_empty() {
        let _ = writeln!(
            out,
            "│ {bold_magenta}Triggers:{reset}    {}",
            s.triggers.join(", ")
        );
    }
    if let Some(desc) = &s.description {
        let _ = writeln!(out, "│ {bold_magenta}Description:{reset} {desc}");
    }
    let _ = writeln!(
        out,
        "╰─────────────────────────────────────────────────────────────\n"
    );
    let _ = writeln!(
        out,
        "{dim}--- Content Preview ---{reset}\n{}",
        detail.content
    );
    out
}

pub fn render_mcp_table(registrations: &[&McpRegistration]) -> String {
    if registrations.is_empty() {
        return "No MCP registrations found.\n".to_owned();
    }
    let headers = [
        "SELECTOR",
        "STATE",
        "TRANSPORT",
        "TARGET",
        "TOOLS",
        "SOURCE",
    ];
    let rows: Vec<Vec<String>> = registrations
        .iter()
        .map(|registration| {
            let state = if !registration.valid {
                "invalid"
            } else if registration.enabled {
                "enabled"
            } else {
                "disabled"
            };
            let target = registration
                .command
                .as_deref()
                .or(registration.url.as_deref())
                .unwrap_or("-");
            let tools = if registration.tool_policy.include.is_empty() {
                "runtime unknown".to_owned()
            } else {
                format!("configured: {}", registration.tool_policy.include.len())
            };
            vec![
                registration.selector.clone(),
                state.to_owned(),
                registration.transport.as_str().to_owned(),
                target.to_owned(),
                tools,
                registration.source.display().to_string(),
            ]
        })
        .collect();
    strip_terminal_controls(&render_table(&headers, &rows))
}

pub fn render_mcp_detail(detail: &McpDetail) -> String {
    render_mcp_registration_detail(&detail.registration, detail.probe.as_ref())
}

pub fn render_mcp_registration_detail(
    registration: &McpRegistration,
    probe: Option<&McpProbe>,
) -> String {
    let mut out = String::new();
    write_mcp_registration(&mut out, registration);
    match probe {
        None => {
            let _ = writeln!(out, "Runtime metadata: not probed");
        }
        Some(probe) => write_mcp_probe(&mut out, probe),
    }
    strip_terminal_controls(&out)
}

fn write_mcp_registration(out: &mut String, registration: &McpRegistration) {
    let _ = writeln!(out, "MCP {}", registration.selector);
    let _ = writeln!(out, "Static registration metadata");
    let _ = writeln!(out, "  Provider:  {}", registration.provider);
    let _ = writeln!(out, "  Scope:     {}", registration.scope);
    let _ = writeln!(out, "  Source:    {}", registration.source.display());
    let _ = writeln!(out, "  Format:    {:?}", registration.source_format);
    let _ = writeln!(out, "  State:     {}", mcp_state(registration));
    let _ = writeln!(out, "  Transport: {}", registration.transport.as_str());
    if let Some(display_name) = &registration.display_name {
        let _ = writeln!(out, "  Display:   {display_name}");
    }
    if let Some(description) = &registration.description {
        let _ = writeln!(out, "  Description: {description}");
    }
    write_mcp_connection(out, registration);
    write_mcp_registration_policy(out, registration);
    for warning in &registration.warnings {
        let _ = writeln!(out, "  Warning: {warning}");
    }
}

fn write_mcp_connection(out: &mut String, registration: &McpRegistration) {
    if let Some(command) = &registration.command {
        let _ = writeln!(out, "  Command:   {command}");
    }
    if !registration.args.is_empty() {
        let _ = writeln!(out, "  Args:      {}", registration.args.join(" "));
    }
    if let Some(url) = &registration.url {
        let _ = writeln!(out, "  URL:       {url}");
    }
    if let Some(cwd) = &registration.cwd {
        let _ = writeln!(out, "  CWD:       {}", cwd.display());
    }
    if !registration.authentication.is_empty() {
        let values = registration
            .authentication
            .iter()
            .map(|auth| {
                auth.reference.as_ref().map_or_else(
                    || auth.kind.clone(),
                    |reference| format!("{} ({reference})", auth.kind),
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let _ = writeln!(out, "  Authentication: {values}");
    }
    write_bindings(out, "Environment", &registration.environment);
    write_bindings(out, "Headers", &registration.headers);
    let timeouts = [
        ("startup", registration.timeouts.startup_ms),
        ("catalog", registration.timeouts.catalog_ms),
        ("tool", registration.timeouts.tool_ms),
    ]
    .into_iter()
    .filter_map(|(name, value)| value.map(|value| format!("{name}={value}ms")))
    .collect::<Vec<_>>();
    if !timeouts.is_empty() {
        let _ = writeln!(out, "  Timeouts:  {}", timeouts.join(", "));
    }
}

fn write_mcp_registration_policy(out: &mut String, registration: &McpRegistration) {
    if !registration.tool_policy.include.is_empty() {
        let _ = writeln!(
            out,
            "  Configured tools: {}",
            registration.tool_policy.include.join(", ")
        );
    }
    if !registration.tool_policy.exclude.is_empty() {
        let _ = writeln!(
            out,
            "  Excluded tools:   {}",
            registration.tool_policy.exclude.join(", ")
        );
    }
    if !registration.extra_fields.is_empty() {
        let _ = writeln!(
            out,
            "  Unnormalized keys: {}",
            registration.extra_fields.join(", ")
        );
    }
}

fn write_mcp_probe(out: &mut String, probe: &McpProbe) {
    let _ = writeln!(
        out,
        "Runtime metadata: {} ({}ms)",
        probe.status, probe.duration_ms
    );
    if let Some(protocol) = &probe.protocol_version {
        let _ = writeln!(out, "  Protocol: {protocol}");
    }
    if let Some(server) = &probe.server {
        let identity = server.title.as_deref().unwrap_or(&server.name);
        let _ = writeln!(out, "  Server:   {identity} {}", server.version);
        if let Some(description) = &server.description {
            let _ = writeln!(out, "  Description: {description}");
        }
        if let Some(website) = &server.website_url {
            let _ = writeln!(out, "  Website: {website}");
        }
    }
    if let Some(capabilities) = &probe.capabilities {
        write_mcp_capabilities(out, capabilities);
    }
    if let Some(instructions) = &probe.instructions {
        let _ = writeln!(out, "  Instructions: {instructions}");
    }
    write_mcp_runtime_catalogs(out, probe);
    for warning in &probe.warnings {
        let _ = writeln!(out, "  Warning: {warning}");
    }
    if let Some(error) = &probe.error {
        let _ = writeln!(out, "  Error: {error}");
    }
}

fn write_mcp_capabilities(out: &mut String, capabilities: &McpServerCapabilities) {
    let mut names = Vec::new();
    if capabilities.tools.is_some() {
        names.push("tools");
    }
    if capabilities.prompts.is_some() {
        names.push("prompts");
    }
    if capabilities.resources.is_some() {
        names.push("resources");
    }
    if capabilities.logging {
        names.push("logging");
    }
    if capabilities.completions {
        names.push("completions");
    }
    if capabilities.experimental {
        names.push("experimental");
    }
    let value = if names.is_empty() {
        "none advertised".to_owned()
    } else {
        names.join(", ")
    };
    let _ = writeln!(out, "  Capabilities: {value}");
    if !capabilities.extensions.is_empty() {
        let _ = writeln!(out, "  Extensions: {}", capabilities.extensions.join(", "));
    }
}

fn write_mcp_runtime_catalogs(out: &mut String, probe: &McpProbe) {
    if !probe.tools.is_empty() {
        let _ = writeln!(out, "Runtime tools: {}", probe.tools.len());
        for tool in &probe.tools {
            let state = if tool.enabled_by_registration {
                "enabled"
            } else {
                "filtered"
            };
            let _ = writeln!(out, "  - {} [{state}]", tool.name);
            if let Some(description) = &tool.description {
                let _ = writeln!(out, "    {description}");
            }
        }
    }
    if !probe.prompts.is_empty() {
        let _ = writeln!(out, "Runtime prompts: {}", probe.prompts.len());
        for prompt in &probe.prompts {
            let _ = writeln!(out, "  - {}", prompt.name);
            if let Some(description) = &prompt.description {
                let _ = writeln!(out, "    {description}");
            }
        }
    }
    if !probe.resources.is_empty() {
        let _ = writeln!(out, "Runtime resources: {}", probe.resources.len());
        for resource in &probe.resources {
            let _ = writeln!(out, "  - {} ({})", resource.name, resource.uri);
        }
    }
    if !probe.resource_templates.is_empty() {
        let _ = writeln!(
            out,
            "Runtime resource templates: {}",
            probe.resource_templates.len()
        );
        for template in &probe.resource_templates {
            let _ = writeln!(out, "  - {} ({})", template.name, template.uri_template);
        }
    }
}

const fn mcp_state(registration: &McpRegistration) -> &'static str {
    if !registration.valid {
        "invalid"
    } else if registration.enabled {
        "enabled"
    } else {
        "disabled"
    }
}

fn write_bindings(out: &mut String, label: &str, bindings: &[crate::mcp::McpValueBinding]) {
    if bindings.is_empty() {
        return;
    }
    let values = bindings
        .iter()
        .map(|binding| format!("{} ({:?})", binding.name, binding.source))
        .collect::<Vec<_>>()
        .join(", ");
    let _ = writeln!(out, "  {label}: {values}");
}

fn strip_terminal_controls(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character == '\n' || character == '\t' || !character.is_control() {
                character
            } else {
                '\u{fffd}'
            }
        })
        .collect()
}

fn render_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers
        .iter()
        .map(|header| UnicodeWidthStr::width(*header))
        .collect();
    for row in rows {
        for (index, value) in row.iter().enumerate() {
            widths[index] = widths[index].max(UnicodeWidthStr::width(value.as_str()));
        }
    }

    let mut output = String::new();
    for (index, header) in headers.iter().enumerate() {
        if index > 0 {
            output.push_str("  ");
        }
        write_cell(&mut output, header, widths[index]);
    }
    output.push('\n');
    for row in rows {
        for (index, value) in row.iter().enumerate() {
            if index > 0 {
                output.push_str("  ");
            }
            write_cell(&mut output, value, widths[index]);
        }
        output.push('\n');
    }
    output
}

fn write_cell(output: &mut String, value: &str, width: usize) {
    output.push_str(value);
    let padding = width.saturating_sub(UnicodeWidthStr::width(value));
    let _ = write!(output, "{:padding$}", "");
}

fn project_label(path: &std::path::Path) -> String {
    path.file_name()
        .filter(|name| !name.is_empty())
        .map_or_else(
            || path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
}

fn format_age(updated_at: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(updated_at, |duration| duration.as_secs());
    format!("{} ago", format_duration(now.saturating_sub(updated_at)))
}

pub fn format_duration(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = seconds % 86_400 / 3_600;
    let minutes = seconds % 3_600 / 60;
    let seconds = seconds % 60;
    if days > 0 {
        format!("{days}d {hours:02}h")
    } else if hours > 0 {
        format!("{hours}h {minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

pub fn format_duration_millis(milliseconds: u64) -> String {
    if milliseconds < 1_000 {
        return format!("{milliseconds}ms");
    }
    let total_tenths = milliseconds.saturating_add(50) / 100;
    let total_seconds = total_tenths / 10;
    let tenths = total_tenths % 10;
    let hours = total_seconds / 3_600;
    let minutes = total_seconds % 3_600 / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours}h {minutes:02}m {seconds:02}.{tenths}s")
    } else if minutes > 0 {
        format!("{minutes}m {seconds:02}.{tenths}s")
    } else {
        format!("{seconds}.{tenths}s")
    }
}

pub fn format_count(value: u64) -> String {
    let digits = value.to_string();
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            formatted.push(',');
        }
        formatted.push(digit);
    }
    formatted
}

pub fn format_token_breakdown(usage: TokenUsage) -> Option<String> {
    let mut parts = Vec::new();
    for (label, value) in [
        ("input", usage.input),
        ("output", usage.output),
        ("cache read", usage.cache_read),
    ] {
        if let Some(value) = value {
            parts.push(format!("{label} {}", format_count(value)));
        }
    }
    if let Some(cache_write) = usage.cache_write {
        let mut detail = Vec::new();
        for (label, value) in [("5m", usage.cache_write_5m), ("1h", usage.cache_write_1h)] {
            if let Some(value) = value {
                detail.push(format!("{label} {}", format_count(value)));
            }
        }
        let detail = if detail.is_empty() {
            String::new()
        } else {
            format!(" ({})", detail.join(" · "))
        };
        parts.push(format!("cache write {}{detail}", format_count(cache_write)));
    }
    for (label, value) in [("reasoning", usage.reasoning), ("tool", usage.tool)] {
        if let Some(value) = value {
            parts.push(format!("{label} {}", format_count(value)));
        }
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

pub fn format_response_header_metrics(response: &ResponseMetrics) -> Vec<String> {
    let mut parts = Vec::new();
    if let Some(duration_ms) = response.duration_ms {
        parts.push(format_duration_millis(duration_ms));
    }
    if let Some(tokens) = response.tokens.total {
        parts.push(format!("{} tokens", format_count(tokens)));
    }
    if let Some(cost) = response.cost_usd {
        parts.push(format!("${cost:.4}"));
    }
    parts
}

pub fn format_response_summary(response: &ResponseMetrics) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(status) = response_status(response) {
        parts.push(format!("status {status}"));
    }
    if let Some(reason) = response.finish_reason.as_deref() {
        parts.push(format!("stop reason {reason}"));
    }
    if let Some(ttft) = response.time_to_first_token_ms {
        parts.push(format!("TTFT {}", format_duration_millis(ttft)));
    }
    if let Some(retries) = response.retry_count {
        parts.push(format!("retries {}", format_count(retries)));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

pub fn format_tool_summary(tool: &ToolMetrics) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(status) = tool.status.as_deref() {
        parts.push(status.to_owned());
    }
    if let Some(duration_ms) = tool.duration_ms {
        parts.push(format_duration_millis(duration_ms));
    }
    if let Some(exit_code) = tool.exit_code {
        parts.push(format!("exit {exit_code}"));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

pub fn format_metric_error(error: &MetricError) -> String {
    error.code.as_deref().map_or_else(
        || error.message.clone(),
        |code| format!("{code} · {}", error.message),
    )
}

pub fn format_model_usage_summary(summary: &ModelUsageSummary) -> String {
    let mut parts = vec![
        summary.model.clone(),
        format!("{} responses", format_count(summary.responses)),
    ];
    if let Some(duration_ms) = summary.duration_ms {
        parts.push(format!("duration {}", format_duration_millis(duration_ms)));
    }
    if let Some(ttft) = summary.average_time_to_first_token_ms {
        parts.push(format!("avg TTFT {}", format_duration_millis(ttft)));
    }
    if let Some(tokens) = summary.tokens.total {
        parts.push(format!("{} tokens", format_count(tokens)));
    }
    if let Some(cost) = summary.cost_usd {
        parts.push(format!("${cost:.4}"));
    }
    if let Some(retries) = summary.retry_count {
        parts.push(format!("{} retries", format_count(retries)));
    }
    if summary.errors > 0 {
        parts.push(format!("{} errors", format_count(summary.errors)));
    }
    parts.join(" · ")
}

pub const TOOL_TOKEN_ACCOUNTING_NOTE: &str =
    "Token accounting: provider response totals only; no per-call token value is persisted.";

fn response_status(response: &ResponseMetrics) -> Option<&'static str> {
    if response.error.is_some() {
        return Some("error");
    }
    let reason = response.finish_reason.as_deref()?.to_ascii_lowercase();
    match reason.as_str() {
        "task_complete" | "complete" | "completed" | "stop" | "end_turn" | "success" => {
            Some("completed")
        }
        "tool_use" | "tool_calls" | "tool-calls" | "function_call" => Some("tool use"),
        "max_tokens" | "length" | "length_limit" => Some("length limit"),
        "turn_aborted" | "aborted" | "interrupt" | "interrupted" | "user_interrupt" => {
            Some("interrupted")
        }
        "error" | "failed" | "failure" => Some("error"),
        "cancelled" | "canceled" => Some("cancelled"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use super::{
        format_count, format_duration_millis, format_metric_error, format_response_header_metrics,
        format_response_summary, format_token_breakdown, format_tool_summary, render_mcp_detail,
        render_mcp_table, render_process_table,
    };
    use crate::mcp::{
        McpDetail, McpRegistration, McpSourceFormat, McpTimeouts, McpToolPolicy, McpTransport,
    };
    use crate::process::{AgentKind, LiveAgent, ProcessSnapshot};
    use crate::session::{MetricError, ResponseMetrics, TokenUsage, ToolMetrics};

    #[test]
    fn formats_exact_counts_and_subsecond_response_durations() {
        assert_eq!(format_count(123_456_789), "123,456,789");
        assert_eq!(format_duration_millis(450), "450ms");
        assert_eq!(format_duration_millis(12_345), "12.3s");
        assert_eq!(format_duration_millis(125_450), "2m 05.5s");
        assert_eq!(format_duration_millis(3_725_450), "1h 02m 05.5s");
    }

    #[test]
    fn process_table_hides_commands_unless_verbose() {
        let agent = LiveAgent {
            kind: AgentKind::Codex,
            process: ProcessSnapshot {
                pid: 42,
                parent_pid: Some(1),
                executable: PathBuf::from("/opt/bin/codex"),
                command: vec!["codex".to_owned(), "--api-key=secret".to_owned()],
                cwd: Some(PathBuf::from("/work/project")),
                started_at: 1,
                run_time: 125,
                cpu_percent: 0.0,
                memory_bytes: 0,
                status: "sleeping".to_owned(),
            },
        };

        let default = render_process_table(std::slice::from_ref(&agent), false);
        assert!(default.contains("codex"));
        assert!(default.contains("2m 05s"));
        assert!(!default.contains("COMMAND"));
        assert!(!default.contains("secret"));

        let verbose = render_process_table(&[agent], true);
        assert!(verbose.contains("COMMAND"));
        assert!(verbose.contains("codex --api-key=secret"));
    }

    #[test]
    fn empty_process_table_is_a_success_message() {
        assert_eq!(
            render_process_table(&[], false),
            "No running developer-agent processes found.\n"
        );
    }

    #[test]
    fn formats_only_persisted_token_breakdown_fields() {
        assert_eq!(
            format_token_breakdown(TokenUsage {
                total: Some(100),
                input: Some(70),
                output: Some(30),
                cache_read: Some(0),
                cache_write: None,
                cache_write_5m: None,
                cache_write_1h: None,
                reasoning: None,
                tool: None,
            })
            .as_deref(),
            Some("input 70 · output 30 · cache read 0")
        );
        assert_eq!(
            format_token_breakdown(TokenUsage {
                total: Some(100),
                ..TokenUsage::default()
            }),
            None
        );
    }

    #[test]
    fn formats_response_and_tool_metrics_without_inventing_missing_values() {
        let error = MetricError {
            code: Some("rate_limit".to_owned()),
            message: "retry budget exhausted".to_owned(),
        };
        let response = ResponseMetrics {
            duration_ms: Some(12_345),
            time_to_first_token_ms: Some(450),
            cost_usd: Some(0.125),
            finish_reason: Some("error".to_owned()),
            retry_count: Some(2),
            error: Some(error.clone()),
            tokens: TokenUsage {
                total: Some(67_890),
                ..TokenUsage::default()
            },
        };

        assert_eq!(
            format_response_header_metrics(&response),
            ["12.3s", "67,890 tokens", "$0.1250"]
        );
        assert_eq!(
            format_response_summary(&response).as_deref(),
            Some("status error · stop reason error · TTFT 450ms · retries 2")
        );
        assert_eq!(
            format_metric_error(&error),
            "rate_limit · retry budget exhausted"
        );
        assert_eq!(
            format_tool_summary(&ToolMetrics {
                status: Some("completed".to_owned()),
                duration_ms: Some(140),
                exit_code: Some(0),
                error: None,
            })
            .as_deref(),
            Some("completed · 140ms · exit 0")
        );
        assert_eq!(format_tool_summary(&ToolMetrics::default()), None);
    }

    #[test]
    fn renders_static_mcp_inventory_without_claiming_runtime_tools() {
        let registration = mcp_registration();
        let table = render_mcp_table(&[&registration]);
        assert!(table.contains("codex:user:docs"));
        assert!(table.contains("stdio"));
        assert!(table.contains("configured: 1"));

        let detail = render_mcp_detail(&McpDetail {
            registration,
            probe: None,
        });
        assert!(detail.contains("Static registration metadata"));
        assert!(detail.contains("Runtime metadata: not probed"));
        assert!(!detail.contains("Runtime tools: 1"));
    }

    #[test]
    fn mcp_text_rendering_neutralizes_terminal_control_sequences() {
        let mut registration = mcp_registration();
        registration.description = Some("safe\u{1b}[2Jcontent".to_owned());
        let output = render_mcp_detail(&McpDetail {
            registration,
            probe: None,
        });
        assert!(!output.contains('\u{1b}'));
        assert!(output.contains("safe�[2Jcontent"));
    }

    fn mcp_registration() -> McpRegistration {
        McpRegistration {
            selector: "codex:user:docs".to_owned(),
            name: "docs".to_owned(),
            provider: "codex".to_owned(),
            scope: "user".to_owned(),
            source: PathBuf::from("/tmp/config.toml"),
            source_format: McpSourceFormat::Toml,
            transport: McpTransport::Stdio,
            enabled: true,
            valid: true,
            display_name: None,
            description: Some("Documentation search".to_owned()),
            command: Some("docs-server".to_owned()),
            args: vec!["--safe".to_owned()],
            url: None,
            cwd: None,
            timeouts: McpTimeouts::default(),
            authentication: Vec::new(),
            environment: Vec::new(),
            headers: Vec::new(),
            tool_policy: McpToolPolicy {
                include: vec!["search".to_owned()],
                ..McpToolPolicy::default()
            },
            options: BTreeMap::new(),
            extra_fields: Vec::new(),
            warnings: Vec::new(),
        }
    }
}
