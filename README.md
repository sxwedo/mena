# mena

`mena` is a fast, local-first control plane for developer-agent processes and
native sessions. It discovers running agents, correlates them with persisted
provider sessions, exposes exact recorded usage, and safely controls or resumes
them without a daemon or remote service.

## Capabilities

- Discover Claude Code, Codex, Gemini CLI, OpenCode, Pi, Oh My Pi, Cursor Agent,
  and configured custom agents.
- Inspect live processes or archived sessions with stable selectors.
- Watch CPU, memory, persisted token usage, and recorded cost in a responsive TUI.
- List and search sessions even when their process is no longer running.
- Read a bounded, redacted event tail or explicitly opt into raw JSONL records.
- Resume sessions through each provider's native argv without invoking a shell.
- Stop processes only after PID, start time, executable, and provider identity are
  revalidated.
- Permanently delete native session artifacts after explicit confirmation while
  refusing to delete a session attached to a running process.

All inspection happens on the local machine. `mena` never uploads process or
conversation data and never estimates cost from public model prices.

## Installation

```sh
git clone https://github.com/sxwedo/mena.git
cd mena
mise run build
install -m 0755 target/release/mena /usr/local/bin/mena
```

Rust 1.96.1 is pinned in `.mise.toml`.

## Migrating from clix

The Agent command family now belongs exclusively to mena:

| Previous command | Mena command |
|---|---|
| `clix agent ps` / `clix-agent ps` | `mena ps` |
| `clix agent top` / `clix-agent top` | `mena top` |
| `clix agent inspect` / `clix-agent inspect` | `mena inspect` |
| `clix agent logs` / `clix-agent logs` | `mena logs` |
| `clix agent sessions` / `clix-agent sessions` | `mena sessions` |
| `clix agent stop` / `clix-agent stop` | `mena stop` |
| `clix agent resume` / `clix-agent resume` | `mena resume` |

The old commands have been removed from clix rather than retained as aliases.
Native provider session files stay in place and require no conversion.

## Commands

### Running processes

```sh
# List recognized live agents. IDs can be passed to inspect, logs, and stop.
mena ps
mena ps --json

# Interactive CPU/memory view. j/k or arrows select, Enter/i opens details,
# r refreshes, and q exits. Non-terminals render bounded snapshots.
mena top
mena top --interval 3 --iterations 5
```

### Saved sessions

```sh
# Interactive manager on a terminal; use --plain or --json for stable output.
mena sessions
mena sessions --provider claude --limit 20
mena sessions --plain
mena sessions --json

# Inspect a process or archived session.
mena inspect codex:12345
mena inspect codex:019abcde-session-id

# Read a bounded event tail; --raw exposes the selected original records.
mena logs claude:session-id -n 100
mena logs claude:session-id -n 20 --raw
```

The interactive session manager supports `/` search, `Enter`/`i` to open a
large session-detail popup, `r` resume, and `d` followed by `y` for permanent
deletion. The detail popup shows complete session metadata and the full native
chat transcript. While it is open, arrows or `j`/`k` scroll the transcript,
`PgUp`/`PgDn` page, `Home`/`End` jump, `e` exports the complete detail as
Markdown, and `Enter` or `Esc` returns to the session list without changing its
selection. Message headers are bold and color-coded: user green, assistant
cyan, tool call yellow, tool result magenta, system/meta dark gray, and error
red. Message bodies keep the terminal's default foreground color.

Exports are created in the directory where `mena` was started and the popup
shows the resulting absolute path. Names use
`mena-session-{provider}-{safe-id}-{YYYYMMDD-HHMMSS}.md`; repeated exports in
the same second add `-2`, `-3`, and so on without replacing existing files.
Writes are atomic and leave no partial output on failure. Exported files use
mode `0600` on Unix; other platforms retain the atomic, no-overwrite behavior
without promising Unix permission bits.

### Process control and resume

```sh
# Graceful termination after process identity revalidation; --force kills.
mena stop claude:12345
mena stop claude:12345 --force

# Pick a session interactively, resume the newest, or list resumable sessions.
mena resume
mena resume --last
mena resume --list --limit 20

# Resume directly through a provider's native CLI.
mena resume codex:session-id
mena resume omp:session-id
mena resume cursor:chat-id
```

## Provider support

| Provider | Process discovery | Native session catalog | Resume | Permanent deletion |
|---|:---:|:---:|:---:|:---:|
| Claude Code | ✓ | ✓ | ✓ | ✓ |
| Codex | ✓ | ✓ | ✓ | ✓ |
| Gemini CLI | ✓ | ✓ | ✓ | ✓ |
| OpenCode | ✓ | ✓ | ✓ | ✓ |
| Pi | ✓ | ✓ | ✓ | ✓ |
| Oh My Pi | ✓ | ✓ | ✓ | ✓ |
| Cursor Agent | ✓ | — | ✓ | — |
| Custom agent | ✓ | — | configurable | — |

Cursor and custom agents do not expose a supported native local session catalog.
Unsupported archived-session operations fail explicitly instead of guessing a
path.

## Configuration

Create a private config file:

```sh
mena config init
```

This creates `~/.config/mena/config.toml` with mode `0600` on Unix. The base
directory honors `XDG_CONFIG_HOME`.

Custom agents use exact executable basenames plus optional argv markers. Resume
commands are argv arrays with a required `{session}` placeholder:

```toml
[agent.custom.my_agent]
executables = ["my-agent", "my-agent.exe"]
command_contains = ["--agent-mode"]
resume = ["my-agent", "resume", "{session}"]
```

Then use the same stable selectors:

```sh
mena inspect my_agent:12345
mena stop my_agent:12345
mena resume my_agent:session-id
```

### Migrating custom agents from clix

If custom recognizers still live in `~/.config/clix/config.toml`, import only its
`[agent.custom]` section into the new mena config:

```sh
mena config init --import-clix
```

Provider sessions do not need to be copied. They remain in each provider's
native store and mena reads them in place.

## Output and safety contracts

The process table exposes `ID`, `AGENT`, `PROJECT`, `STATUS`, `DURATION`,
`TOKENS`, and `COST`; `top` adds CPU and memory. A `-` means no native session
could be associated. `n/a` means a session was found but the provider did not
persist an exact cost.

Session titles use native metadata when available and otherwise fall back to the
first user message. Usage is parsed only for the selected or associated session
and cached until its backing file changes. Log reads bound both individual
record size and retained tail size.

Deletion is provider-aware. It removes duplicate catalog paths plus known
sidecars and native indexes, validates every storage identifier, rejects paths
that escape provider-owned roots, and refuses deletion for a live session. The
operation is permanent and requires an explicit lowercase `y` confirmation.

## Architecture

```text
src/
├── main.rs        # `mena` CLI entrypoint and exit handling
├── lib.rs         # command definitions and dispatch
├── controller.rs  # command orchestration, targeting, JSON, resume argv
├── export.rs      # complete Markdown rendering and collision-safe export
├── process.rs     # process discovery, recognition, identity-safe stop
├── session.rs     # provider catalogs, usage parsing, logs, safe deletion
├── tui.rs         # responsive process and session interfaces
├── view.rs        # stable plain-text tables and formatting
├── settings.rs    # ~/.config/mena config and clix custom-agent import
├── fs.rs          # atomic replacement and private no-overwrite creation
└── ui.rs          # terminal status and error presentation
```

## Development

```sh
mise run verify  # fmt + check + test + strict clippy + rustdoc
mise run build   # release binary
```

Run one test with:

```sh
cargo test <name_substring> -- --nocapture
```

## License

[MIT](LICENSE) © 2026 sxwedo
