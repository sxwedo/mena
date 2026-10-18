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
