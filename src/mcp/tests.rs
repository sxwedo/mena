use std::fs;

use anyhow::Result;
use tempfile::tempdir;

use super::{McpCatalog, McpTransport, McpValueSource};

#[test]
fn basic_edit_updates_only_the_selected_codex_registration() -> Result<()> {
    let home = tempdir()?;
    fs::create_dir_all(home.path().join(".codex"))?;
    let path = home.path().join(".codex/config.toml");
    fs::write(
        &path,
        r#"# keep this file comment
[features]
experimental = true

[mcp_servers.docs]
# keep this server comment
command = "npx"
args = ["-y", "@example/docs"]
cwd = "/old/workspace"
enabled = true
env = { API_TOKEN = "keep-secret" }

[mcp_servers.other]
command = "other-server"
"#,
    )?;

    let catalog = McpCatalog::scan(Some(home.path()), None)?;
    let registration = catalog
        .registrations()
        .iter()
        .find(|registration| registration.name == "docs")
        .expect("docs registration");
    let updated = catalog.update_basic_config(
        registration,
        &super::McpConfigPatch {
            command: Some(Some("bunx".to_owned())),
            args: Some(vec!["@example/docs@latest".to_owned()]),
            cwd: Some(None),
            ..super::McpConfigPatch::default()
        },
    )?;

    assert_eq!(updated.command.as_deref(), Some("bunx"));
    assert_eq!(updated.args, ["@example/docs@latest"]);
    assert_eq!(updated.cwd, None);

    let content = fs::read_to_string(path)?;
    assert!(content.contains("# keep this file comment"));
    assert!(content.contains("# keep this server comment"));
    assert!(content.contains("API_TOKEN = \"keep-secret\""));
    assert!(content.contains("[mcp_servers.other]"));
    assert!(content.contains("command = \"other-server\""));
    Ok(())
}

#[test]
fn source_line_and_delete_target_only_the_selected_codex_table() -> Result<()> {
    let home = tempdir()?;
    fs::create_dir_all(home.path().join(".codex"))?;
    let path = home.path().join(".codex/config.toml");
    fs::write(
        &path,
        r#"# keep this comment
[mcp_servers.keep]
command = "keep-server"

[mcp_servers.remove]
command = "remove-server"

[mcp_servers.remove.env]
TOKEN = "keep-private"
"#,
    )?;

    let catalog = McpCatalog::scan(Some(home.path()), None)?;
    let registration = catalog
        .registrations()
        .iter()
        .find(|registration| registration.name == "remove")
        .expect("remove registration");
    assert_eq!(catalog.source_line(registration)?, 5);

    catalog.delete_registration(registration)?;

    let content = fs::read_to_string(&path)?;
    assert!(content.contains("# keep this comment"));
    assert!(content.contains("[mcp_servers.keep]"));
    assert!(!content.contains("mcp_servers.remove"));
    assert!(!content.contains("keep-private"));
    Ok(())
}

#[test]
fn json_delete_cleans_native_policy_lists_and_preserves_other_servers() -> Result<()> {
    let home = tempdir()?;
    fs::create_dir_all(home.path().join(".gemini"))?;
    let path = home.path().join(".gemini/settings.json");
    fs::write(
        &path,
        r#"{
  "mcpServers": {
    "keep": {"command": "keep-server"},
    "remove": {"command": "remove-server"}
  },
  "mcp": {
    "allowed": ["keep", "remove"],
    "excluded": ["remove"]
  }
}"#,
    )?;

    let catalog = McpCatalog::scan(Some(home.path()), None)?;
    let registration = catalog
        .registrations()
        .iter()
        .find(|registration| registration.name == "remove")
        .expect("remove registration");
    assert_eq!(catalog.source_line(registration)?, 4);

    catalog.delete_registration(registration)?;

    let root: serde_json::Value = serde_json::from_str(&fs::read_to_string(path)?)?;
    assert!(root["mcpServers"].get("keep").is_some());
    assert!(root["mcpServers"].get("remove").is_none());
    assert_eq!(root["mcp"]["allowed"], serde_json::json!(["keep"]));
    assert_eq!(root["mcp"]["excluded"], serde_json::json!([]));
    Ok(())
}

#[test]
fn source_line_follows_native_jsonc_and_yaml_object_paths() -> Result<()> {
    let home = tempdir()?;
    let opencode_path = home.path().join(".config/opencode/opencode.jsonc");
    fs::create_dir_all(opencode_path.parent().expect("OpenCode parent"))?;
    fs::write(
        &opencode_path,
        r#"{
  mcp: {
    decoy: {
      selected: { command: ["wrong"] }
    },
    selected: {
      type: "local",
      command: ["right"]
    }
  }
}"#,
    )?;
    let goose_path = home.path().join(".config/goose/config.yaml");
    fs::create_dir_all(goose_path.parent().expect("Goose parent"))?;
    fs::write(
        &goose_path,
        r"selected: wrong
extensions:
  decoy:
    type: builtin
    selected:
      type: builtin
  selected:
    type: builtin
",
    )?;

    let catalog = McpCatalog::scan(Some(home.path()), None)?;
    let opencode = catalog
        .registrations()
        .iter()
        .find(|registration| registration.provider == "opencode" && registration.name == "selected")
        .expect("OpenCode registration");
    let goose = catalog
        .registrations()
        .iter()
        .find(|registration| registration.provider == "goose" && registration.name == "selected")
        .expect("Goose registration");

    assert_eq!(catalog.source_line(opencode)?, 6);
    assert_eq!(catalog.source_line(goose)?, 7);
    Ok(())
}

#[test]
fn source_line_falls_back_to_start_when_an_external_edit_is_temporarily_invalid() -> Result<()> {
    let home = tempdir()?;
    fs::create_dir_all(home.path().join(".codex"))?;
    let path = home.path().join(".codex/config.toml");
    fs::write(&path, "[mcp_servers.docs]\ncommand = \"docs-server\"\n")?;
    let catalog = McpCatalog::scan(Some(home.path()), None)?;
    let registration = catalog
        .registrations()
        .first()
        .expect("registration")
        .clone();

    fs::write(&path, "[temporarily invalid")?;

    assert_eq!(catalog.source_line(&registration)?, 1);
    Ok(())
}

#[test]
fn empty_basic_edit_patch_rediscovers_external_changes() -> Result<()> {
    let home = tempdir()?;
    fs::create_dir_all(home.path().join(".codex"))?;
    let path = home.path().join(".codex/config.toml");
    fs::write(
        &path,
        r#"[mcp_servers.docs]
command = "old-server"
"#,
    )?;

    let catalog = McpCatalog::scan(Some(home.path()), None)?;
    let registration = catalog
        .registrations()
        .iter()
        .find(|registration| registration.name == "docs")
        .expect("docs registration");
    fs::write(
        &path,
        r#"[mcp_servers.docs]
command = "new-server"
"#,
    )?;

    let refreshed = catalog.update_basic_config(registration, &super::McpConfigPatch::default())?;

    assert_eq!(refreshed.command.as_deref(), Some("new-server"));
    Ok(())
}

#[test]
fn basic_edit_preserves_unrelated_json_configuration() -> Result<()> {
    let home = tempdir()?;
    fs::create_dir_all(home.path().join(".cursor"))?;
    let path = home.path().join(".cursor/mcp.json");
    fs::write(
        &path,
        r#"{
  "theme": "keep-root-setting",
  "mcpServers": {
    "docs": {
      "command": "npx",
      "args": ["old"],
      "cwd": "/old/workspace",
      "enabled": true,
      "env": {"API_TOKEN": "keep-secret"},
      "custom": {"keep": true}
    },
    "other": {"command": "other-server"}
  }
}"#,
    )?;

    let catalog = McpCatalog::scan(Some(home.path()), None)?;
    let registration = catalog
        .registrations()
        .iter()
        .find(|registration| registration.selector == "cursor:user:docs")
        .expect("docs registration");
    let updated = catalog.update_basic_config(
        registration,
        &super::McpConfigPatch {
            command: Some(Some("bunx".to_owned())),
            args: Some(vec!["@example/docs@latest".to_owned()]),
            cwd: Some(None),
            ..super::McpConfigPatch::default()
        },
    )?;

    assert_eq!(updated.command.as_deref(), Some("bunx"));
    assert_eq!(updated.args, ["@example/docs@latest"]);
    assert_eq!(updated.cwd, None);

    let content: serde_json::Value = serde_json::from_str(&fs::read_to_string(path)?)?;
    assert_eq!(content["theme"], "keep-root-setting");
    assert_eq!(
        content["mcpServers"]["docs"]["env"]["API_TOKEN"],
        "keep-secret"
    );
    assert_eq!(content["mcpServers"]["docs"]["custom"]["keep"], true);
    assert_eq!(content["mcpServers"]["other"]["command"], "other-server");
    Ok(())
}

#[test]
fn basic_edit_uses_opencode_native_command_array() -> Result<()> {
    let home = tempdir()?;
    fs::create_dir_all(home.path().join(".config/opencode"))?;
    let path = home.path().join(".config/opencode/opencode.json");
    fs::write(
        &path,
        r#"{
  "mcp": {
    "servers": {
      "docs": {
        "type": "local",
        "command": ["npx", "-y", "old-package"],
        "disabled": false,
        "environment": {"API_TOKEN": "keep-secret"}
      }
    }
  }
}"#,
    )?;

    let catalog = McpCatalog::scan(Some(home.path()), None)?;
    let registration = catalog
        .registrations()
        .iter()
        .find(|registration| registration.selector == "opencode:user:docs")
        .expect("docs registration");
    let updated = catalog.update_basic_config(
        registration,
        &super::McpConfigPatch {
            enabled: Some(false),
            command: Some(Some("bunx".to_owned())),
            args: Some(vec!["new-package".to_owned(), "--stdio".to_owned()]),
            ..super::McpConfigPatch::default()
        },
    )?;

    assert!(!updated.enabled);
    assert_eq!(updated.command.as_deref(), Some("bunx"));
    assert_eq!(updated.args, ["new-package", "--stdio"]);
    let content: serde_json::Value = serde_json::from_str(&fs::read_to_string(path)?)?;
    assert_eq!(
        content["mcp"]["servers"]["docs"]["command"],
        serde_json::json!(["bunx", "new-package", "--stdio"])
    );
    assert_eq!(content["mcp"]["servers"]["docs"]["disabled"], true);
    assert_eq!(
        content["mcp"]["servers"]["docs"]["environment"]["API_TOKEN"],
        "keep-secret"
    );
    Ok(())
}

#[test]
fn basic_edit_targets_the_nearest_claude_local_project() -> Result<()> {
    let home = tempdir()?;
    let project = home.path().join("work/project");
    let nested = project.join("nested/current");
    fs::create_dir_all(&nested)?;
    let path = home.path().join(".claude.json");
    let source = serde_json::json!({
        "projects": {
            project.to_string_lossy(): {
                "mcpServers": {
                    "docs": {"command": "parent-server"}
                }
            },
            project.join("nested").to_string_lossy(): {
                "mcpServers": {
                    "docs": {"command": "nested-server"}
                }
            }
        }
    });
    fs::write(&path, serde_json::to_vec_pretty(&source)?)?;

    let catalog = McpCatalog::scan(Some(home.path()), Some(&nested))?;
    let registration = catalog
        .registrations()
        .iter()
        .find(|registration| registration.selector == "claude:local:docs")
        .expect("local docs registration");
    assert_eq!(registration.command.as_deref(), Some("nested-server"));

    let updated = catalog.update_basic_config(
        registration,
        &super::McpConfigPatch {
            command: Some(Some("updated-nested-server".to_owned())),
            ..super::McpConfigPatch::default()
        },
    )?;
    assert_eq!(updated.command.as_deref(), Some("updated-nested-server"));

    let content: serde_json::Value = serde_json::from_str(&fs::read_to_string(path)?)?;
    assert_eq!(
        content["projects"][project.to_string_lossy().as_ref()]["mcpServers"]["docs"]["command"],
        "parent-server"
    );
    assert_eq!(
        content["projects"][project.join("nested").to_string_lossy().as_ref()]["mcpServers"]["docs"]
            ["command"],
        "updated-nested-server"
    );
    Ok(())
}

#[test]
fn basic_edit_updates_omp_native_enabled_lists() -> Result<()> {
    let home = tempdir()?;
    fs::create_dir_all(home.path().join(".omp/agent"))?;
    let path = home.path().join(".omp/agent/mcp.json");
    fs::write(
        &path,
        r#"{
  "mcpServers": {"docs": {"command": "docs-server"}},
  "enabledServers": ["docs", "other"],
  "disabledServers": ["legacy"]
}"#,
    )?;

    let catalog = McpCatalog::scan(Some(home.path()), None)?;
    let registration = catalog
        .registrations()
        .iter()
        .find(|registration| registration.selector == "omp:user:docs")
        .expect("OMP docs registration");
    let updated = catalog.update_basic_config(
        registration,
        &super::McpConfigPatch {
            enabled: Some(false),
            ..super::McpConfigPatch::default()
        },
    )?;

    assert!(!updated.enabled);
    let content: serde_json::Value = serde_json::from_str(&fs::read_to_string(path)?)?;
    assert_eq!(content["enabledServers"], serde_json::json!(["other"]));
    assert_eq!(
        content["disabledServers"],
        serde_json::json!(["legacy", "docs"])
    );
    Ok(())
}

#[test]
fn basic_edit_updates_gemini_allowed_and_excluded_policy() -> Result<()> {
    let home = tempdir()?;
    fs::create_dir_all(home.path().join(".gemini"))?;
    let path = home.path().join(".gemini/settings.json");
    fs::write(
        &path,
        r#"{
  "mcpServers": {
    "docs": {"command": "docs-server"},
    "other": {"command": "other-server"}
  },
  "mcp": {
    "allowed": ["docs"],
    "excluded": []
  }
}"#,
    )?;

    let catalog = McpCatalog::scan(Some(home.path()), None)?;
    let registration = catalog
        .registrations()
        .iter()
        .find(|registration| registration.selector == "gemini:user:docs")
        .expect("Gemini docs registration");
    let disabled = catalog.update_basic_config(
        registration,
        &super::McpConfigPatch {
            enabled: Some(false),
            ..super::McpConfigPatch::default()
        },
    )?;
    assert!(!disabled.enabled);
    let content: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path)?)?;
    assert_eq!(content["mcp"]["allowed"], serde_json::json!(["docs"]));
    assert_eq!(content["mcp"]["excluded"], serde_json::json!(["docs"]));

    let enabled = catalog.update_basic_config(
        &disabled,
        &super::McpConfigPatch {
            enabled: Some(true),
            ..super::McpConfigPatch::default()
        },
    )?;
    assert!(enabled.enabled);
    let content: serde_json::Value = serde_json::from_str(&fs::read_to_string(path)?)?;
    assert_eq!(content["mcp"]["allowed"], serde_json::json!(["docs"]));
    assert_eq!(content["mcp"]["excluded"], serde_json::json!([]));
    Ok(())
}

#[test]
fn basic_edit_refuses_jsonc_without_rewriting_comments() -> Result<()> {
    let home = tempdir()?;
    fs::create_dir_all(home.path().join(".config/opencode"))?;
    let path = home.path().join(".config/opencode/opencode.jsonc");
    let original = r#"{
  // This comment must never be normalized away.
  "mcp": {
    "docs": {
      "type": "local",
      "command": ["docs-server"]
    }
  }
}"#;
    fs::write(&path, original)?;

    let catalog = McpCatalog::scan(Some(home.path()), None)?;
    let registration = catalog
        .registrations()
        .iter()
        .find(|registration| registration.selector == "opencode:user:docs")
        .expect("OpenCode docs registration");
    let error = catalog
        .update_basic_config(
            registration,
            &super::McpConfigPatch {
                command: Some(Some("new-server".to_owned())),
                ..super::McpConfigPatch::default()
            },
        )
        .expect_err("JSONC must require the external editor");

    assert!(error.to_string().contains("press `o`"));
    assert_eq!(fs::read_to_string(path)?, original);
    Ok(())
}

#[test]
fn basic_edit_never_writes_redaction_placeholders_over_secrets() -> Result<()> {
    let home = tempdir()?;
    fs::create_dir_all(home.path().join(".codex"))?;
    let path = home.path().join(".codex/config.toml");
    let original = r#"[mcp_servers.secure]
command = "secure-server"
args = ["--api-key", "literal-secret"]
"#;
    fs::write(&path, original)?;

    let catalog = McpCatalog::scan(Some(home.path()), None)?;
    let registration = catalog
        .registrations()
        .iter()
        .find(|registration| registration.name == "secure")
        .expect("secure registration");
    assert!(
        registration
            .args
            .iter()
            .any(|argument| argument == "<redacted>")
    );
    let error = catalog
        .update_basic_config(
            registration,
            &super::McpConfigPatch {
                args: Some(registration.args.clone()),
                ..super::McpConfigPatch::default()
            },
        )
        .expect_err("redacted values must never be persisted");

    assert!(error.to_string().contains("still contains `<redacted>`"));
    assert_eq!(fs::read_to_string(path)?, original);
    Ok(())
}

#[test]
fn codex_stdio_registration_is_normalized_without_exposing_secrets() -> Result<()> {
    let home = tempdir()?;
    fs::create_dir_all(home.path().join(".codex"))?;
    fs::write(
        home.path().join(".codex/config.toml"),
        r#"
[mcp_servers.docs]
command = "npx"
args = ["-y", "@example/docs-mcp", "--api-key", "super-secret", "--header", "Authorization: Bearer header-secret", "https://alice:password@example.test/mcp?token=url-secret"]
env = { API_TOKEN = "literal-secret" }
env_vars = ["FORWARDED_TOKEN", { name = "LOCAL_EXTRA", source = "local" }]
startup_timeout_sec = 20.5
tool_timeout_sec = 45
required = true
enabled_tools = ["search", "read"]
disabled_tools = ["write"]
default_tools_approval_mode = "prompt"

[mcp_servers.docs.tools.search]
approval_mode = "approve"
"#,
    )?;

    let catalog = McpCatalog::scan(Some(home.path()), None)?;
    let registrations = catalog.registrations();
    assert_eq!(registrations.len(), 1);
    let server = &registrations[0];
    assert_eq!(server.selector, "codex:user:docs");
    assert_eq!(server.transport, McpTransport::Stdio);
    assert_eq!(server.command.as_deref(), Some("npx"));
    assert_eq!(
        server.args,
        [
            "-y",
            "@example/docs-mcp",
            "--api-key",
            "<redacted>",
            "--header",
            "<redacted>",
            "https://example.test/mcp?token=<redacted>",
        ]
    );
    assert_eq!(server.timeouts.startup_ms, Some(20_500));
    assert_eq!(server.timeouts.tool_ms, Some(45_000));
    assert_eq!(server.tool_policy.include, ["read", "search"]);
    assert_eq!(server.tool_policy.exclude, ["write"]);
    assert_eq!(
        server.tool_policy.approval_overrides.get("search"),
        Some(&"approve".to_owned())
    );

    let output = serde_json::to_string(server)?;
    assert!(!output.contains("super-secret"));
    assert!(!output.contains("header-secret"));
    assert!(!output.contains("url-secret"));
    assert!(!output.contains("literal-secret"));
    assert!(output.contains("API_TOKEN"));
    assert!(output.contains("FORWARDED_TOKEN"));
    assert!(output.contains("LOCAL_EXTRA"));
    let debug = format!("{catalog:?}");
    for secret in [
        "super-secret",
        "header-secret",
        "url-secret",
        "literal-secret",
    ] {
        assert!(!debug.contains(secret));
    }
    Ok(())
}

#[test]
fn codex_http_registration_records_auth_and_header_sources_safely() -> Result<()> {
    let home = tempdir()?;
    let workspace = tempdir()?;
    fs::create_dir_all(workspace.path().join(".codex"))?;
    fs::write(
        workspace.path().join(".codex/config.toml"),
        r#"
[mcp_servers.remote]
url = "https://alice:password@example.com/mcp?token=secret&region=cn#private"
enabled = false
bearer_token_env_var = "REMOTE_MCP_TOKEN"
http_headers = { "X-Tenant" = "tenant-secret" }
env_http_headers = { "X-Api-Key" = "REMOTE_API_KEY" }
startup_timeout_ms = 1250
scopes = ["catalog.read", "catalog.write"]
oauth_resource = "https://alice:password@example.com/audience?tenant=secret"
"#,
    )?;

    let catalog = McpCatalog::scan(Some(home.path()), Some(workspace.path()))?;
    let server = catalog.registrations().first().expect("one server");
    assert_eq!(server.selector, "codex:project:remote");
    assert_eq!(server.transport, McpTransport::StreamableHttp);
    assert!(!server.enabled);
    assert_eq!(server.timeouts.startup_ms, Some(1_250));
    assert_eq!(
        server.url.as_deref(),
        Some("https://example.com/mcp?token=<redacted>&region=<redacted>")
    );
    assert_eq!(
        server
            .authentication
            .iter()
            .map(|auth| auth.kind.as_str())
            .collect::<Vec<_>>(),
        ["bearer_env", "oauth"]
    );
    assert_eq!(
        server.options.get("oauth_scopes"),
        Some(&serde_json::json!(["catalog.read", "catalog.write"]))
    );
    assert_eq!(
        server.options.get("oauth_resource"),
        Some(&serde_json::json!(
            "https://example.com/audience?tenant=<redacted>"
        ))
    );
    assert_eq!(
        server
            .headers
            .iter()
            .map(|header| (header.name.as_str(), header.source))
            .collect::<Vec<_>>(),
        [
            ("X-Api-Key", super::McpValueSource::Environment),
            ("X-Tenant", super::McpValueSource::Literal),
        ]
    );
    let output = serde_json::to_string(server)?;
    for secret in ["password", "secret", "tenant-secret"] {
        assert!(!output.contains(secret));
    }
    Ok(())
}

#[test]
fn catalog_filters_and_requires_disambiguation_for_duplicate_names() -> Result<()> {
    let home = tempdir()?;
    let workspace = tempdir()?;
    fs::create_dir_all(home.path().join(".codex"))?;
    fs::create_dir_all(workspace.path().join(".codex"))?;
    fs::write(
        home.path().join(".codex/config.toml"),
        "[mcp_servers.docs]\ncommand = \"user-docs\"\n",
    )?;
    fs::write(
        workspace.path().join(".codex/config.toml"),
        "[mcp_servers.docs]\ncommand = \"project-docs\"\n",
    )?;

    let catalog = McpCatalog::scan(Some(home.path()), Some(workspace.path()))?;
    let error = catalog
        .inspect("docs", None, None, None)
        .expect_err("a bare duplicate name must be rejected");
    let message = error.to_string();
    assert!(message.contains("ambiguous MCP registration `docs`"));
    assert!(message.contains("codex:user:docs"));
    assert!(message.contains("codex:project:docs"));

    let detail = catalog.inspect("docs", Some("codex"), Some("project"), None)?;
    assert_eq!(detail.registration.command.as_deref(), Some("project-docs"));
    assert!(detail.probe.is_none());

    let selected = catalog.select(Some("codex"), None, Some(".codex/config.toml"))?;
    assert_eq!(selected.len(), 2);
    assert!(
        catalog
            .select(Some("unknown"), None, None)
            .expect_err("unknown providers must be actionable")
            .to_string()
            .contains("unsupported MCP provider")
    );
    Ok(())
}

#[test]
fn discovers_json_client_registrations_across_user_local_and_project_scopes() -> Result<()> {
    let home = tempdir()?;
    let workspace = home.path().join("work/project");
    fs::create_dir_all(&workspace)?;
    fs::create_dir_all(home.path().join(".cursor"))?;
    fs::create_dir_all(home.path().join(".gemini"))?;
    fs::create_dir_all(home.path().join(".config/opencode"))?;

    let workspace_key = workspace.to_string_lossy().replace('\\', "\\\\");
    fs::write(
        home.path().join(".claude.json"),
        format!(
            r#"{{
  "mcpServers": {{
    "claude-user": {{"command": "claude-user", "env": {{"TOKEN": "hidden"}}}}
  }},
  "projects": {{
    "{workspace_key}": {{
      "mcpServers": {{"claude-local": {{"type": "http", "url": "https://example.test/mcp"}}}}
    }}
  }}
}}"#
        ),
    )?;
    fs::write(
        workspace.join(".mcp.json"),
        r#"{"mcpServers":{"claude-project":{"command":"project-server"}}}"#,
    )?;
    fs::write(
        home.path().join(".cursor/mcp.json"),
        r#"{"mcpServers":{"cursor-docs":{"url":"https://cursor.test/mcp","headers":{"Authorization":"secret"}}}}"#,
    )?;
    fs::write(
        home.path().join(".gemini/settings.json"),
        r#"{"mcpServers":{"gemini-search":{"httpUrl":"https://gemini.test/mcp","includeTools":["search"],"excludeTools":["delete"],"timeout":20000,"trust":true,"futureFlag":"present"}}}"#,
    )?;
    fs::write(
        home.path().join(".config/opencode/opencode.jsonc"),
        r#"{
              // OpenCode v1 configuration
              "mcp": {
                "opencode-local": {
                  "type": "local",
                  "command": ["npx", "-y", "server"],
                  "environment": {"API_KEY": "opencode-secret"},
                  "enabled": true
                }
              }
            }"#,
    )?;

    let catalog = McpCatalog::scan(Some(home.path()), Some(&workspace))?;
    let selectors: Vec<&str> = catalog
        .registrations()
        .iter()
        .map(|registration| registration.selector.as_str())
        .collect();
    assert_eq!(
        selectors,
        [
            "claude:local:claude-local",
            "claude:project:claude-project",
            "claude:user:claude-user",
            "cursor:user:cursor-docs",
            "gemini:user:gemini-search",
            "opencode:user:opencode-local",
        ]
    );
    let gemini = catalog
        .registrations()
        .iter()
        .find(|registration| registration.name == "gemini-search")
        .expect("Gemini registration");
    assert_eq!(gemini.transport, McpTransport::StreamableHttp);
    assert_eq!(gemini.timeouts.catalog_ms, Some(20_000));
    assert_eq!(gemini.tool_policy.include, ["search"]);
    assert_eq!(gemini.tool_policy.exclude, ["delete"]);
    assert_eq!(gemini.extra_fields, ["futureFlag"]);

    let output = serde_json::to_string(catalog.registrations())?;
    for secret in ["hidden", "opencode-secret", "Authorization\":\"secret"] {
        assert!(!output.contains(secret));
    }
    Ok(())
}

#[test]
fn discovers_goose_omp_and_opted_in_pi_adapter_metadata() -> Result<()> {
    let home = tempdir()?;
    let workspace = home.path().join("work/project");
    fs::create_dir_all(&workspace)?;
    fs::create_dir_all(home.path().join(".config/goose"))?;
    fs::create_dir_all(home.path().join(".omp/agent"))?;
    fs::create_dir_all(workspace.join(".omp"))?;
    fs::create_dir_all(home.path().join(".pi/agent"))?;
    fs::create_dir_all(home.path().join(".config/mcp"))?;
    fs::write(
        home.path().join(".config/goose/config.yaml"),
        r"extensions:
  goose-tools:
    name: Goose Tools
    description: Local tool catalog
    enabled: true
    type: stdio
    cmd: goose-mcp
    args: [serve]
    env_keys: [GOOSE_TOKEN]
    available_tools: [lookup, summarize]
    timeout: 30
",
    )?;
    fs::write(
        home.path().join(".omp/agent/mcp.json"),
        r#"{
              "mcpServers": {
                "omp-remote": {
                  "type": "http",
                  "url": "https://omp.test/mcp",
                  "headers": {"Authorization": "!security find-generic-password"}
                },
                "omp-off": {"command": "off-server"}
              },
              "disabledServers": ["omp-off"]
            }"#,
    )?;
    fs::write(
        workspace.join(".omp/mcp.json"),
        r#"{"mcpServers":{"omp-project":{"command":"project-omp"}}}"#,
    )?;
    fs::write(
        home.path().join(".pi/agent/settings.json"),
        r#"{"packages":["github:example/pi-mcp-adapter"]}"#,
    )?;
    fs::write(
        home.path().join(".pi/agent/mcp.json"),
        r#"{"mcpServers":{"pi-user":{"command":"pi-server"}}}"#,
    )?;
    fs::write(
        home.path().join(".config/mcp/mcp.json"),
        r#"{"mcpServers":{"pi-shared":{"url":"https://pi.test/mcp"}}}"#,
    )?;

    let catalog = McpCatalog::scan(Some(home.path()), Some(&workspace))?;
    let registrations = catalog.registrations();
    for selector in [
        "goose:user:goose-tools",
        "omp:project:omp-project",
        "omp:user:omp-off",
        "omp:user:omp-remote",
        "pi:shared:pi-shared",
        "pi:user:pi-user",
    ] {
        assert!(
            registrations
                .iter()
                .any(|registration| registration.selector == selector),
            "missing {selector}"
        );
    }
    let goose = registrations
        .iter()
        .find(|registration| registration.name == "goose-tools")
        .expect("Goose extension");
    assert_eq!(goose.display_name.as_deref(), Some("Goose Tools"));
    assert_eq!(goose.tool_policy.include, ["lookup", "summarize"]);
    assert_eq!(goose.timeouts.catalog_ms, Some(30_000));
    assert_eq!(goose.environment[0].name, "GOOSE_TOKEN");
    assert_eq!(goose.environment[0].source, McpValueSource::Forwarded);

    let omp_off = registrations
        .iter()
        .find(|registration| registration.name == "omp-off")
        .expect("disabled OMP server");
    assert!(!omp_off.enabled);
    let omp_remote = registrations
        .iter()
        .find(|registration| registration.name == "omp-remote")
        .expect("remote OMP server");
    assert_eq!(omp_remote.headers[0].source, McpValueSource::DynamicCommand);
    assert!(
        omp_remote
            .warnings
            .iter()
            .any(|warning| warning.contains("never executed"))
    );
    assert!(!serde_json::to_string(registrations)?.contains("find-generic-password"));
    Ok(())
}

#[test]
fn explicit_probe_refuses_disabled_registration_without_starting_it() -> Result<()> {
    let home = tempdir()?;
    fs::create_dir_all(home.path().join(".codex"))?;
    let sentinel = home.path().join("must-not-exist");
    fs::write(
        home.path().join(".codex/config.toml"),
        format!(
            "[mcp_servers.disabled]\ncommand = \"sh\"\nargs = [\"-c\", \"touch {}\"]\nenabled = false\n",
            sentinel.display()
        ),
    )?;

    let catalog = McpCatalog::scan(Some(home.path()), None)?;
    let detail = catalog.inspect_with_probe("disabled", None, None, None, 1)?;
    let probe = detail.probe.expect("probe decision");
    assert_eq!(probe.status, super::McpProbeStatus::Refused);
    assert!(
        probe
            .error
            .as_deref()
            .is_some_and(|error| error.contains("disabled"))
    );
    assert!(
        !sentinel.exists(),
        "disabled command was unexpectedly started"
    );
    Ok(())
}

#[test]
fn explicit_probe_refuses_dynamic_value_commands_without_executing_them() -> Result<()> {
    let home = tempdir()?;
    fs::create_dir_all(home.path().join(".omp/agent"))?;
    let sentinel = home.path().join("dynamic-command-must-not-run");
    fs::write(
        home.path().join(".omp/agent/mcp.json"),
        format!(
            r#"{{"mcpServers":{{"dynamic":{{
                    "type":"http",
                    "url":"http://127.0.0.1:9/mcp",
                    "headers":{{"Authorization":"!touch {}"}}
                }}}}}}"#,
            sentinel.display()
        ),
    )?;

    let catalog = McpCatalog::scan(Some(home.path()), None)?;
    let detail = catalog.inspect_with_probe("dynamic", Some("omp"), None, None, 1)?;
    let probe = detail.probe.expect("probe decision");
    assert_eq!(probe.status, super::McpProbeStatus::Failed);
    assert!(
        probe
            .error
            .as_deref()
            .is_some_and(|error| error.contains("dynamic value"))
    );
    assert!(!sentinel.exists(), "dynamic value command was executed");
    Ok(())
}

#[test]
fn explicit_probe_does_not_run_remote_codex_stdio_locally() -> Result<()> {
    let home = tempdir()?;
    fs::create_dir_all(home.path().join(".codex"))?;
    let sentinel = home.path().join("remote-command-must-not-run-locally");
    fs::write(
        home.path().join(".codex/config.toml"),
        format!(
            "[mcp_servers.remote]\ncommand = \"sh\"\nargs = [\"-c\", \"touch {}\"]\nexperimental_environment = \"remote\"\nenv_vars = [{{ name = \"REMOTE_TOKEN\", source = \"remote\" }}]\n",
            sentinel.display()
        ),
    )?;

    let catalog = McpCatalog::scan(Some(home.path()), None)?;
    let registration = &catalog.registrations()[0];
    assert_eq!(
        registration.options.get("execution_environment"),
        Some(&serde_json::json!("remote"))
    );
    assert!(registration.environment.iter().any(|binding| {
        binding.name == "REMOTE_TOKEN" && binding.source == super::McpValueSource::RemoteEnvironment
    }));
    let detail = catalog.inspect_with_probe("remote", Some("codex"), None, None, 1)?;
    let probe = detail.probe.expect("probe decision");
    assert_eq!(probe.status, super::McpProbeStatus::Unsupported);
    assert!(
        probe
            .error
            .as_deref()
            .is_some_and(|error| error.contains("remote executor"))
    );
    assert!(!sentinel.exists(), "remote command was started locally");
    Ok(())
}

#[test]
fn discovers_only_enabled_claude_and_codex_plugin_mcp_servers() -> Result<()> {
    let home = tempdir()?;
    let workspace = home.path().join("work/project");
    let claude_plugin = home.path().join(".claude/plugins/cache/acme/db/1.0.0");
    let codex_marketplace = home.path().join("codex-marketplace");
    let codex_plugin = codex_marketplace.join("plugins/computer-use");
    fs::create_dir_all(claude_plugin.join(".claude-plugin"))?;
    fs::create_dir_all(codex_plugin.join(".codex-plugin"))?;
    fs::create_dir_all(home.path().join(".claude/plugins"))?;
    fs::create_dir_all(home.path().join(".codex"))?;
    fs::create_dir_all(&workspace)?;
    fs::write(
        home.path().join(".claude/settings.json"),
        r#"{"enabledPlugins":{"db@acme":true,"off@acme":false}}"#,
    )?;
    fs::write(
        home.path().join(".claude/plugins/installed_plugins.json"),
        format!(
            r#"{{"version":2,"plugins":{{"db@acme":[{{"scope":"user","installPath":{},"version":"1.0.0"}}]}}}}"#,
            serde_json::to_string(&claude_plugin)?
        ),
    )?;
    fs::write(
        claude_plugin.join(".claude-plugin/plugin.json"),
        r#"{"name":"db","mcpServers":"./mcp.json"}"#,
    )?;
    fs::write(
        claude_plugin.join("mcp.json"),
        r#"{"mcpServers":{"plugin-db":{"type":"http","url":"https://plugin.test/mcp","headers":{"Authorization":"Bearer ${PLUGIN_TOKEN}"}}}}"#,
    )?;
    fs::write(
        codex_plugin.join(".codex-plugin/plugin.json"),
        r#"{"name":"computer-use"}"#,
    )?;
    fs::write(
        codex_plugin.join(".mcp.json"),
        r#"{"computer-use":{"command":"./bin/server","args":["mcp"],"cwd":".","env_vars":["DISPLAY"]}}"#,
    )?;
    fs::write(
        home.path().join(".codex/config.toml"),
        format!(
            r#"[marketplaces.bundle]
source_type = "local"
source = {}

[plugins."computer-use@bundle"]
enabled = true

[plugins."off@bundle"]
enabled = false
"#,
            toml::Value::String(codex_marketplace.to_string_lossy().into_owned())
        ),
    )?;

    let catalog = McpCatalog::scan(Some(home.path()), Some(&workspace))?;
    let plugins: Vec<_> = catalog
        .registrations()
        .iter()
        .filter(|registration| registration.scope == "plugin")
        .collect();
    assert_eq!(plugins.len(), 2);
    assert_eq!(plugins[0].selector, "claude:plugin:plugin-db");
    assert_eq!(
        plugins[0].options.get("plugin_id"),
        Some(&serde_json::json!("db@acme"))
    );
    assert_eq!(plugins[0].headers[0].source, McpValueSource::Environment);
    assert_eq!(plugins[1].selector, "codex:plugin:computer-use");
    let codex_plugin_canonical = codex_plugin.canonicalize()?;
    assert_eq!(
        plugins[1].cwd.as_deref(),
        Some(codex_plugin_canonical.as_path())
    );
    assert_eq!(plugins[1].environment[0].name, "DISPLAY");
    assert_eq!(plugins[1].environment[0].source, McpValueSource::Forwarded);
    Ok(())
}
