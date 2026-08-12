# MCP catalog

`mena mcp` answers two different questions without mixing their evidence:

1. **What is registered?** The default command reads local client configuration
   files and enabled plugin manifests. It does not connect to a server or start
   a process.
2. **What does the server expose now?** Pressing `p` in the browser or using
   `inspect --probe` explicitly initializes one server and requests its
   advertised catalogs. It never invokes a tool, reads resource content, or
   expands a prompt.

## Commands

```sh
mena mcp
mena mcp --json
mena mcp --provider codex --scope project
mena mcp --source .codex/config.toml

mena mcp --provider codex inspect codegraph
mena mcp --provider codex inspect codegraph --json
mena mcp --provider codex inspect codegraph --probe --timeout 15
```

With an interactive terminal, `mena mcp` opens a searchable two-pane browser.
The initial detail is entirely static. The same command writes a plain table
when piped, while `--json` always produces machine-readable output.

Browser keys:

- `/` filters name, selector, client, scope, transport, target, description,
  configured tools, and source;
- `↑`/`↓` or `j`/`k` moves the list, then scrolls detail after switching panes;
- `Tab`, `←`/`→`, or `h`/`l` switches panes without changing selection;
- `Enter` toggles a full-screen inspector;
- `p` explicitly probes the selected registration in a worker thread, so list
  browsing remains responsive; the result stays attached to that registration
  even if selection moves;
- `q`/`Esc` goes back or exits. If a probe is running, mena waits for its bounded
  cleanup before exiting so a stdio child is not abandoned.

List filters are applied before inspection:

- `--provider`: `claude`, `codex`, `cursor`, `gemini`, `goose`, `omp`,
  `opencode`, or `pi`.
- `--scope`: `user`, `local`, `project`, `plugin`, `profile`, `managed`, or
  `shared`.
- `--source`: an exact configuration path or path suffix.
- `--json`: stable machine-readable output.

The normal selector is `provider:scope:name`. Mena deliberately records every
source instead of applying a client's precedence rules. If two sources still
produce the same selector, inspection fails as ambiguous and lists the paths;
add `--source` to select the intended registration.

This is an audit of declarations, not a reconstruction of a client's final
merged runtime state. Client trust rules, managed policy, command-line
overrides, or a running client's cached state can still suppress a recorded
entry; `enabled` is the state declared by that source.

## Configuration sources

Project discovery walks from the current directory toward the home directory
and uses the nearest matching project file.

| Client | Scope | Sources recorded |
|---|---|---|
| Claude Code | user | `~/.claude.json` → `mcpServers` |
| Claude Code | local | `~/.claude.json` → nearest matching `projects.<path>.mcpServers` |
| Claude Code | project | nearest `.mcp.json` |
| Claude Code | plugin | MCP manifests from installed **and enabled** plugins only |
| Claude Code | managed | the current platform's `managed-mcp.json` system path |
| Codex | user | `~/.codex/config.toml` → `[mcp_servers.*]` |
| Codex | project | nearest `.codex/config.toml` → `[mcp_servers.*]` |
| Codex | plugin | `.mcp.json` from enabled plugins in local marketplaces |
| Cursor | user / project | `~/.cursor/mcp.json`, nearest `.cursor/mcp.json` |
| Gemini CLI | user / project | `~/.gemini/settings.json`, nearest `.gemini/settings.json` |
| OpenCode | user / project | `~/.config/opencode/opencode.json[.c]`, nearest `opencode.json[.c]`; v1 and v2 shapes |
| Goose | user | `~/.config/goose/config.yaml` → `extensions` |
| Oh My Pi | user / project | `~/.omp/agent/mcp.json`, nearest `.omp/mcp.json` |
| Oh My Pi | profile | `~/.omp/profiles/*/agent/mcp.json` |
| Pi adapter | user / shared / project | `~/.pi/agent/mcp.json`, `~/.config/mcp/mcp.json`, nearest `.pi/mcp.json` / `.mcp.json` |

Pi itself has no native MCP registry. Mena scans Pi paths only when
`~/.pi/agent/settings.json` shows a `pi-mcp-adapter` package, so an unrelated
`.mcp.json` is not mislabeled as Pi configuration.

Goose stores external MCP servers and provider-native extensions in one
registry. Mena records its `stdio`, Streamable HTTP, SSE, builtin, platform,
frontend, and inline-Python kinds faithfully. Only external stdio and
Streamable HTTP entries can be live-probed.

Managed Claude configuration is read from:

- macOS: `/Library/Application Support/ClaudeCode/managed-mcp.json`
- Linux/WSL: `/etc/claude-code/managed-mcp.json`
- Windows: `%ProgramFiles%\ClaudeCode\managed-mcp.json`

Remote connectors, app-backed plugins, and server inventories held only in a
client's running memory are not invented as local registrations. They appear
only when a readable local configuration contains a transport definition.

## Static registration metadata

Each registration records:

- identity: selector, name, client provider, scope, source path, and syntax;
- state: enabled, structurally valid, warnings, and unknown field names;
- transport: stdio, Streamable HTTP, SSE, or a provider-native kind;
- launch target: redacted command, argv, URL, working directory, and timeouts;
- authentication metadata: authentication kind and credential reference;
- value bindings: environment/header **names**, source type, and sensitivity
  hint, never their literal values in JSON;
- configured tool policy: include/exclude lists and approval modes;
- safe provider options such as trust, codemode, plugin ID/version, and OMP
  profile name; Codex placement, OAuth resource/scopes, and millisecond or
  fractional-second startup timeouts are normalized as well;
- optional display name and description.

`extra_fields` contains unrecognized configuration **key names** only. Their
values are intentionally omitted because a new client field may contain a
credential.

Known secret-bearing locations are redacted: environment and header values are
omitted, URL userinfo/query values/fragments are removed, bearer tokens are
represented by environment-variable name, and common secret/header argv flags
hide their following value. Names and descriptions remain user-authored data;
do not place credentials in those fields.

## Live protocol metadata

`--probe` adds a `probe` object with:

- status and duration;
- negotiated MCP protocol version;
- server name, title, version, description, website, and instructions;
- tools/prompts/resources capabilities, list-change flags, subscriptions,
  logging, completions, experimental support, and extension identifiers;
- `tools/list`: name, title, description, input/output schemas, safety hints,
  configuration filtering/approval result, and protocol metadata key names;
- `prompts/list`: name, title, description, and argument definitions;
- `resources/list`: URI, name, title, description, MIME type, and size;
- `resources/templates/list`: URI template and descriptive fields;
- partial-list warnings or a sanitized connection/protocol error.

Probe statuses are:

| Status | Meaning |
|---|---|
| `success` | initialization and every advertised list operation succeeded |
| `partial` | initialization succeeded, but a catalog or cleanup step failed |
| `failed` | process, network, authentication, timeout, or protocol failure |
| `refused` | registration is disabled or structurally invalid |
| `unsupported` | static transport exists but mena has no safe live transport |

The `inspect --probe` command prints details before returning a non-zero exit
status for `failed`, `refused`, or `unsupported`. The interactive browser shows
the status in-place and remains open for inspection or retry.

### Probe transport and authentication support

- **stdio:** program and argv are passed directly; no shell is invoked. The
  child receives a small base environment plus explicitly configured or
  forwarded values.
- **Streamable HTTP:** static headers, environment-backed headers, and a bearer
  token environment reference are supported.
- **SSE and provider-native kinds:** recorded statically but not probed.
- **OAuth/provider credential stores:** recorded as authentication metadata,
  but mena does not extract another client's tokens or open an OAuth flow. A
  protected endpoint may therefore return an authentication error.
- **dynamic header/value helpers:** recorded but never executed; live probing
  fails closed until the value is expressed as an environment reference.
- **Codex remote executor placement:** remote environment bindings are marked
  separately, and the registration remains static-only because mena cannot
  reproduce Codex's remote executor safely.

Pressing `p` or using `--probe` executes locally configured code or contacts a
configured endpoint. Review the static inspection first, especially for
project-scoped files.

## Bounds and safety contract

- Configuration reads are capped at 8 MiB and 10,000 registrations per file.
- Installed plugin roots and referenced MCP manifest paths are canonicalized
  and must stay inside the plugin cache or marketplace root.
- Probe timeout is explicit and limited to 1–300 seconds.
- Each runtime catalog is limited to 10,000 entries and 1,000 pages; repeated
  cursors fail instead of looping.
- Text metadata is capped at 64 KiB and individual JSON schemas at 1 MiB.
- Stdio cleanup is bounded and children are killed on transport drop.
- Probe errors are bounded and scrub raw URLs, arguments, headers, and resolved
  credential values before display.
- Server metadata and tool safety annotations are untrusted claims. A probe
  records them but never uses them to authorize execution.

## Implementation map

The MCP feature is a deep module with a small provider-neutral catalog API:

| File | Responsibility |
|---|---|
| `src/lib.rs` | CLI arguments and command dispatch |
| `src/controller.rs` | scan/filter/inspect orchestration and exit behavior |
| `src/tui/mcp/` | search, pane navigation, cached detail, and probe worker |
| `src/mcp.rs` | public models, catalog sorting, filters, resolution, and probe gate |
| `src/mcp/adapter.rs` | closed discovery seam and private connection material |
| `src/mcp/adapter/storage.rs` | bounded reads, nearest-project search, profile limits |
| `src/mcp/adapter/codex.rs` | native TOML normalization |
| `src/mcp/adapter/json_clients.rs` | Claude, Cursor, Gemini, OpenCode, OMP, and Pi formats |
| `src/mcp/adapter/goose.rs` | Goose YAML extension normalization |
| `src/mcp/adapter/plugins.rs` | enabled Claude/Codex plugin discovery and containment |
| `src/mcp/adapter/common.rs` | common normalization, redaction, and raw/public split |
| `src/mcp/probe.rs` | explicit rmcp client, transports, catalog bounds, runtime models |
| `src/view.rs` | human-readable table and detail rendering |

Tests live beside these public seams. They cover registration normalization,
secret redaction (including safe `Debug` output), duplicate resolution, plugin
enablement/containment and wrapped/top-level manifests, dynamic-helper refusal,
remote-executor refusal, disabled-server refusal, and an in-memory MCP server
proving that metadata discovery makes zero tool calls.

## Upstream format references

- [Codex MCP configuration](https://developers.openai.com/codex/mcp/)
- [Claude Code MCP configuration](https://code.claude.com/docs/en/mcp)
- [Gemini CLI MCP configuration](https://google-gemini.github.io/gemini-cli/docs/tools/mcp-server.html)
- [OpenCode MCP configuration](https://opencode.ai/v2/docs/mcp-servers)
- [Oh My Pi MCP configuration](https://github.com/can1357/oh-my-pi/blob/main/docs/mcp-config.md)
- [Goose configuration](https://github.com/aaif-goose/goose/blob/main/documentation/docs/guides/config-files.md)
- [Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk/blob/main/crates/rmcp/README.md)
