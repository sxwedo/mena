# AGENTS.md

This file provides guidance to AI coding agents working in this repository.

## Overview

`mena` is a standalone, local-first Rust CLI for discovering, inspecting, and
controlling developer-agent processes and their native persisted sessions. It
was extracted from `clix-agent`; it must remain independent of the clix
workspace and must not acquire a path dependency back to it.

The user-facing command is `mena`:

```sh
mena agent [provider] [--fresh] [--resume] [--session <id>]
mena ag [provider]
mena sessions
mena skills [--provider <name>] [--scope <scope>]
mena skills inspect <name>
mena config init [--import-clix]
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
      ├── tui/
      │   ├── agent_launcher/
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
- `controller.rs` orchestrates commands, emits stable JSON or text output, and
  resumes through native argv.
- `tui/` owns agent launch, session management, Skill browsing, search, detail
  views, and destructive-action confirmation. TUI modules must not bypass the
  session or Skill catalog seams for provider-owned storage.
- `settings.rs` owns `~/.config/mena/config.toml`. Its optional clix importer is
  a migration boundary only; runtime behavior must not depend on clix.
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

## Configuration

Custom process definitions retain the original schema:

```toml
[agent.custom.my_agent]
executables = ["my-agent"]
command_contains = ["--agent-mode"]
resume = ["my-agent", "resume", "{session}"]
```

`mena config init --import-clix` may read the legacy clix config once to copy
this section. Normal command execution reads mena configuration only.

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
- Preserve saved-session selectors (`provider:session-id`) and JSON field names
  unless making an explicitly versioned breaking change.
- Keep `README.md` and `README_CN.md` concise and synchronized. Put detailed
  behavior in the matching English and Chinese files under `docs/`.
- Update README command examples, the provider matrix, and detailed docs with
  behavior changes.
- Run the full `mise run verify` gate before committing.
