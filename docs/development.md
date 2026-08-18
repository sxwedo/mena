# Architecture and development

`mena` is a standalone Rust 2024 crate pinned to Rust 1.96.1.

## Data flow

```text
main.rs
  └── lib.rs                 command definitions and dispatch
      ├── controller.rs      command orchestration
      ├── continuation.rs    cross-agent import and handoff policy
      ├── process.rs         provider recognition and live evidence
      ├── session.rs         provider-neutral session model and safety
      │   └── session/adapter.rs
      │       ├── storage.rs native layouts and index cleanup
      │       └── detail.rs  transcript normalization
      ├── skill.rs           Skill catalog, selection, and read containment
      │   └── skill/adapter
      │       ├── storage.rs discovery and directory reads
      │       └── detail.rs  bounded text and frontmatter parsing
      ├── mcp.rs             registration model, catalog, and probe gate
      │   ├── adapter.rs     closed client-config discovery seam
      │   │   ├── common.rs  normalization and redaction
      │   │   ├── edit.rs    source location and native config mutations
      │   │   ├── storage.rs bounded configuration I/O
      │   │   ├── codex.rs / json_clients.rs / goose.rs
      │   │   └── plugins.rs enabled-plugin discovery and containment
      │   └── probe.rs       explicit protocol metadata discovery
      ├── tui
      │   ├── agent_launcher
      │   ├── mcp
      │   ├── session
      │   └── skill
      ├── settings.rs
      ├── editor.rs          shell-free external editor launch
      ├── export.rs / clipboard.rs / fs.rs
      └── view.rs / ui.rs
```

Provider session behavior is a closed-enum adapter seam. Built-in providers are
compiled into mena, so adding one makes discovery, association, detail, resume,
and deletion matches non-exhaustive until each behavior is considered.

Cross-agent continuation has a separate seam in `continuation.rs`. It owns the
target matrix, native-import argv, temporary private handoffs, and fresh-target
prompts. The Session TUI returns only `Resume` or `ContinueWith`; it does not
parse transcripts or construct provider commands.

The Skill catalog is the filesystem seam for Skill discovery and preview. TUI
code consumes catalog results and cached directory entries rather than reading
arbitrary paths itself.

The MCP catalog similarly separates public registration metadata from private
connection material. Adapters only read and normalize during a scan. The raw
command, environment, header, and URL values can cross into `probe.rs` only
after the caller explicitly requests a live probe.

The MCP TUI owns grouped search, selection, cached Spotlight detail rendering,
source actions, deletion confirmation, and its bounded probe worker. The worker
calls back through `McpCatalog`; it does not receive or reconstruct private
adapter connection values.

MCP configuration changes cross the same catalog seam. Source-line lookup and
deletion re-read the current native file; deletion validates client-specific
structure and uses `fs.rs` for an atomic permission-preserving write. The
external editor is launched without a shell, and the TUI refreshes through the
Catalog after it exits. The TUI never parses or writes provider configuration
directly.

## Safety invariants

- Construct native resume program and argv separately; never invoke a shell.
- Keep cross-agent routes explicit: native imports use provider-supported flags;
  all other routes create a fresh session from a temporary private handoff.
- Treat resume argv as launch evidence, not current session identity.
- Never infer an exact live association from project equality or recency.
- Show active state only for exact native evidence; protect deletion more
  conservatively when evidence is missing or ambiguous.
- Bound transcript and Skill reads.
- Validate IDs, canonical paths, and provider-root containment before deletion.
- Never estimate token cost or unavailable per-call Tool token values.
- Custom agents have no guessed session catalog.
- Static MCP scans never start a process or contact a server.
- MCP probes never invoke tools, read resources, or render prompts.
- Never invoke a shell or dynamic credential helper for MCP discovery.
- Keep secret-bearing MCP values private; serialize only redacted targets and
  binding names/sources.
- Never persist a redaction placeholder. Managed/plugin sources cannot be
  edited or deleted, and comment-bearing formats cannot be deleted
  automatically.
- MCP configuration writes must re-read the source, preserve unrelated data,
  validate the native shape, and replace the file atomically.
- Treat server descriptions, schemas, and safety annotations as untrusted data.

## Adding provider support

1. Add recognition and native executable behavior in `process.rs`.
2. Extend the closed `ProviderAdapter` enum.
3. Put native paths and index cleanup in `session/adapter/storage.rs`.
4. Normalize native records in `session/adapter/detail.rs`.
5. Add interface-level fixtures for discovery, usage, detail, association,
   ambiguity, deletion, and root containment.
6. Update the README provider matrix and this documentation.

Do not add runtime registration or `dyn Trait` merely to support another
compiled-in provider.

## Adding MCP client support

1. Extend the closed discovery sequence in `mcp/adapter.rs`.
2. Keep native parsing in a focused adapter and normalize through
   `mcp/adapter/common.rs`.
3. Preserve the split between serializable `McpRegistration` and private
   `McpConnection`; never put credential values in a public model.
4. Add interface tests for every native source/scope, ambiguity, unknown
   fields, redaction, environment expansion, and malformed transport.
5. Add live transport support only in `mcp/probe.rs`, with explicit opt-in,
   time/item/page bounds, sanitized errors, and zero tool calls.
6. If the client is writable, add native update behavior and interface tests in
   `mcp/adapter/edit.rs`; do not infer a generic enable field.
7. Update [the MCP source and metadata matrix](mcp.md).

## Verification

All gates are defined in `.mise.toml`:

```sh
mise run fmt
mise run check
mise run test
mise run clippy
mise run docs
mise run verify
mise run build
```

`mise run verify` runs formatting checks, type checking, all tests, pedantic and
nursery Clippy with warnings denied, and rustdoc with warnings denied.
