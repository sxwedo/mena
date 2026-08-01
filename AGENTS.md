# AGENTS.md

This file provides guidance to AI coding agents working in this repository.

## Overview

`mena` is a standalone, local-first Rust CLI for discovering, inspecting, and
controlling developer-agent processes and their native persisted sessions. It
was extracted from `clix-agent`; it must remain independent of the clix
workspace and must not acquire a path dependency back to it.

The user-facing command is `mena`:

```sh
mena ps
mena top
mena inspect <target>
mena logs <target>
mena sessions
mena stop <pid-target>
mena resume [session-target]
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
      │   ├── tui.rs
      │   └── view.rs
      ├── settings.rs
      ├── fs.rs
      └── ui.rs
```

- `process.rs` recognizes built-in and configured custom processes, samples
  resources, and revalidates process identity immediately before signaling.
- `session.rs` scans provider-owned native stores, resolves selectors, parses
  persisted usage, bounds log reads, and performs provider-aware deletion.
- `controller.rs` associates live processes with sessions, emits stable JSON or
  text output, redacts command secrets, and resumes through native argv.
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
- Keep log reads bounded and redact common secret-bearing process arguments in
  default output. `--raw` is an explicit opt-in.
- Never delete a session attached to a running process.
- Validate session IDs and canonical paths before deletion. Traversal and
  symlink escapes outside provider-owned roots must fail closed.
- Keep destructive confirmation explicit and lowercase-`y` only.
- Cursor and custom agents have no generic local session catalog. Do not guess
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

- Add provider session support in `session.rs` with fixtures covering discovery,
  usage, ambiguity, deletion, and root containment.
- Add process recognition in `process.rs`; keep desktop helper processes from
  matching command-line agents.
- Preserve stable selectors (`provider:PID`, `provider:session-id`) and JSON
  field names unless making an explicitly versioned breaking change.
- Update README command examples and the provider matrix with behavior changes.
- Run the full `mise run verify` gate before committing.
