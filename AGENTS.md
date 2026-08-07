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
      │   ├── process.rs
      │   ├── session.rs
      │   │   └── session/adapter.rs
      │   │       ├── detail.rs
      │   │       └── storage.rs
      │   ├── tui.rs
      │   └── view.rs
      ├── settings.rs
      ├── fs.rs
      └── ui.rs
```

- `process.rs` recognizes built-in and configured custom processes, samples
  resources, and revalidates process identity immediately before signaling.
- `session.rs` owns the provider-neutral session model, catalog, selectors,
  evidence-based live association, bounded I/O, and provider-independent
  deletion safeguards.
- `session/adapter.rs` is the single seam for built-in session providers. It
  uses closed-enum static dispatch so discovery, usage, detail, resume, and
  deletion capabilities remain exhaustive without a vtable or runtime
  registry. `adapter/storage.rs` owns native layouts and index cleanup;
  `adapter/detail.rs` normalizes provider records into the shared model.
- `controller.rs` consumes session associations, emits stable JSON or text
  output, redacts command secrets, and resumes through native argv.
- `tui.rs` owns the responsive top view, session picker/manager, search,
  details, and destructive-action confirmation.
- `settings.rs` owns `~/.config/mena/config.toml`. Its optional clix importer is
  a migration boundary only; runtime behavior must not depend on clix.
- `fs.rs` provides atomic, permission-preserving writes for native index repair.

## Safety invariants

- Never signal a PID without revalidating PID, start time, executable, and
  recognized provider against the originally selected process.
- Never invoke a shell for resume commands. Construct program and argv
  separately; substitute only the `{session}` placeholder.
- Never infer token cost from public prices. Report only persisted exact values.
- Never claim a current process-to-session association from project equality,
  recency, or timestamps alone. Only current provider-native runtime evidence is
  exact; a resume argv is launch evidence only. Unconfirmed processes must not
  receive session metrics, and deletion must fail closed for their provider.
- Keep log reads bounded and redact common secret-bearing process arguments in
  default output. `--raw` is an explicit opt-in.
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
- Preserve stable selectors (`provider:PID`, `provider:session-id`) and JSON
  field names unless making an explicitly versioned breaking change.
- Update README command examples and the provider matrix with behavior changes.
- Run the full `mise run verify` gate before committing.
