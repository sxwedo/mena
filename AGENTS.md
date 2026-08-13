# AGENTS.md

This file provides guidance to AI coding agents working in this repository.

## Overview

`mena` is a standalone, local-first Rust CLI for discovering, inspecting, and
controlling developer-agent processes and their native persisted sessions.

The user-facing command is `mena`:

```sh
mena agent [provider] [--fresh] [--resume] [--session <id>]
mena ag [provider]
mena ps [--json] [--verbose]
mena sessions
mena skills [--provider <name>] [--scope <scope>]
mena skills inspect <name>
mena mcp [--provider <client>] [--scope <scope>] [--source <path>] [--json]
mena mcp inspect <name> [--probe] [--timeout <seconds>] [--json]
mena mcp open <name>
mena config init
```

## Verification

All gates are defined in `.mise.toml`:

```sh
mise run verify  # fmt + check + test + strict clippy + rustdoc
mise run fmt
mise run check
mise run test
mise run clippy
mise run docs
mise run build
```

Rust is pinned to 1.96.1 with edition 2024.

## Architecture

Data flows from `main.rs` through command dispatch in `lib.rs` into
`controller.rs`:

```text
main.rs
  └── lib.rs
      ├── controller.rs
      ├── process.rs
      ├── session.rs
      │   └── session/adapter.rs
      │       ├── detail.rs
      │       └── storage.rs
      ├── skill.rs
      │   └── skill/adapter.rs
      │       ├── detail.rs
      │       └── storage.rs
      ├── mcp.rs
      │   ├── adapter.rs
      │   │   ├── common.rs
      │   │   ├── storage.rs
      │   │   ├── codex.rs
      │   │   ├── json_clients.rs
      │   │   ├── goose.rs
      │   │   └── plugins.rs
      │   └── probe.rs
      ├── tui/
      │   ├── agent_launcher/
      │   ├── mcp/
      │   ├── session/
      │   └── skill/
      ├── view.rs
      ├── settings.rs
      ├── fs.rs
      └── ui.rs
```

- `process.rs` recognizes built-in and configured custom processes and obtains
  provider-native evidence needed for safe session protection.
- `session.rs` owns the provider-neutral session model, catalog, selectors,
  evidence-based live association, bounded I/O, and provider-independent
  deletion safeguards.
- `session/adapter.rs` is the single seam for built-in session providers. It
  uses closed-enum static dispatch so discovery, detail, resume, association,
  and deletion capabilities remain exhaustive without a vtable or runtime
  registry. `adapter/storage.rs` owns native layouts and index cleanup;
  `adapter/detail.rs` normalizes provider records into the shared model.
- `skill.rs` is the filesystem seam for Skill discovery, unique selection,
  canonical root containment, bounded preview reads, and directory listing.
- `mcp.rs` owns the provider-neutral MCP catalog, stable filters, ambiguity
  handling, redacted public metadata, safe basic configuration patches, and
  the explicit live-probe gate. `mcp/adapter.rs` is the closed client-config
  seam; raw connection values stay private. `mcp/adapter/edit.rs` owns native
  configuration updates. `mcp/probe.rs` alone may start stdio or contact
  Streamable HTTP, and only after `--probe` or an explicit `p` action in the
  MCP browser.
- `controller.rs` orchestrates commands, emits stable JSON or text output, and
  resumes through native argv.
- `tui/` owns agent launch, session management, Skill and MCP browsing, search,
  detail views, and destructive-action confirmation. TUI modules must not
  bypass the session, Skill, or MCP catalog seams for provider-owned data.
- `settings.rs` owns `~/.config/mena/config.toml` and creates it atomically with
  restrictive permissions.
- `fs.rs` provides atomic, permission-preserving writes for native index repair.

## Safety invariants

- Never invoke a shell for resume commands. Construct program and argv
  separately; substitute only the `{session}` placeholder.
- Never infer token cost from public prices. Report only persisted exact values.
- Never claim a current process-to-session association from project equality,
  recency, or timestamps alone. Only current provider-native runtime evidence is
  exact; a resume argv is launch evidence only. Active display uses exact
  targets, while deletion protection fails closed for the complete provider
  catalog when association is unconfirmed or ambiguous.
- Keep transcript and Skill reads bounded. Skill preview paths must remain
  within the selected canonical Skill root.
- Never delete a session attached to a running process.
- Validate session IDs and canonical paths before deletion. Traversal and
  symlink escapes outside provider-owned roots must fail closed.
- Keep destructive confirmation explicit and lowercase-`y` only.
- Custom agents have no generic local session catalog. Do not guess
  storage paths; return an actionable unsupported-operation error.
- A static MCP scan must never start a configured process, contact an endpoint,
  execute a dynamic value helper, or reuse another client's credential store.
- A live MCP probe is explicit (`p` in the browser or `--probe`) and bounded. It
  may initialize and list protocol metadata, but must never call tools, read
  resource contents, or render prompts.
- Never serialize MCP environment/header/auth values. Redact URL userinfo,
  query values, fragments, and secret-bearing argv positions; sanitize errors.
- Never write redacted values back into MCP files. Re-read and validate the
  native source, preserve unrelated data, and use atomic permission-preserving
  replacement. Plugin/managed sources and formats that cannot retain comments
  must fail closed for embedded editing.
- Treat MCP server metadata and safety annotations as untrusted claims. Keep
  reads, pages, item counts, schemas, text, timeouts, and cleanup bounded.

## Configuration

Custom process definitions retain the original schema:

```toml
[agent.custom.my_agent]
executables = ["my-agent"]
command_contains = ["--agent-mode"]
resume = ["my-agent", "resume", "{session}"]
```

`mena config init` creates the default template without importing configuration
from another application.

## Change guidelines

- Add provider session support through `session/adapter.rs`; keep all provider
  selection inside that module, native layouts in `adapter/storage.rs`, and
  transcript normalization in `adapter/detail.rs`. Extend interface-level
  fixtures for discovery, live-association evidence, usage, full detail,
  ambiguity, deletion, and root containment.
- Prefer the closed-enum adapter while built-in providers are compiled into
  mena. Do not introduce `dyn Trait`, runtime registration, or heap allocation
  merely to add another built-in provider.
- Add process recognition in `process.rs`; keep desktop helper processes from
  matching command-line agents.
- Add Skill storage behavior through `skill.rs` and `skill/adapter/`; do not
  introduce direct filesystem reads into `tui/skill/`.
- Add MCP client formats through `mcp/adapter.rs`, normalize in focused
  adapters, and add interface fixtures for source/scope discovery, ambiguity,
  redaction, environment expansion, plugin containment, and probe paths.
- Keep MCP TUI probing and configuration editing behind `McpCatalog`; do not
  expose private connections or provider-native write logic to `tui/mcp/`.
  Probe results must remain keyed to their originating registration.
- Preserve saved-session selectors (`provider:session-id`) and JSON field names
  unless making an explicitly versioned breaking change.
- Keep `README.md` and `README_CN.md` concise and synchronized. Put detailed
  behavior in the matching English and Chinese files under `docs/`.
- Update README command examples, the provider matrix, and detailed docs with
  behavior changes.
- Run the full `mise run verify` gate before committing.
