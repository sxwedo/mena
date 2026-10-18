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

    // A file this test holds open must be reported (absolute), proving
    // the fd scan actually resolves vnode paths. The kernel reports the
    // canonical path (`/private/var` on macOS), so compare canonically.
    let temp = tempfile::tempdir().expect("temp dir");
    let held = temp.path().join("held-open.txt");
    std::fs::write(&held, "held").expect("write held file");
    let _file = std::fs::File::open(&held).expect("hold file open");
    let expected = held.canonicalize().expect("canonicalize held file");
    let paths = open_file_paths(std::process::id());
    assert!(paths.iter().all(|path| path.is_absolute()));
    assert!(
        paths.contains(&expected),
        "held file must appear in {paths:?}"
    );

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
