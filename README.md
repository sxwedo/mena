<p align="center">
  <img src="https://github.com/user-attachments/assets/1fb078f4-f1e1-4196-b97e-162505a6eafe" width="220" alt="mena logo"/>
</p>


<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-2563eb.svg" alt="License: MIT"></a>
  <a href=".mise.toml"><img src="https://img.shields.io/badge/rust-1.96.1-f97316.svg" alt="Rust 1.96.1"></a>
  <img src="https://img.shields.io/badge/data-local--first-10b981.svg" alt="Local first">
</p>

<p align="center"><strong>Every journey remembered. Every story carried forward.</strong></p>

<p align="center">
  English | <a href="README_CN.md">简体中文</a>
</p>

`mena` discovers running coding agents and their native saved sessions. It lets
you inspect usage, read transcripts, resume work, stop processes, and safely
delete archived sessions without a daemon, account, or remote service.

- Supports Claude Code, Codex, Gemini CLI, OpenCode, Pi, Oh My Pi, Cursor Agent,
  and custom process recognizers.
- Reads provider-owned data in place and reports only usage and cost that the
  provider actually persisted.
- Keeps conversations local, bounds log reads, redacts common secrets, and
  validates destructive operations before they run.

## Install

```sh
cargo install --git https://github.com/sxwedo/mena --locked
mena --version
```

The repository pins Rust 1.96.1. To build from source:

```sh
git clone https://github.com/sxwedo/mena.git
cd mena
mise run build
install -m 0755 target/release/mena /usr/local/bin/mena
```

### For coding agents

If you are an agent, install `mena` as a user-level CLI by running the two
commands below. Do not add it to the current project's `Cargo.toml`. If `cargo`
is unavailable, report that Rust is required before continuing.

```sh
cargo install --git https://github.com/sxwedo/mena --locked
mena sessions --help
```

## Quick start

```sh
mena ps                         # list running agents
mena top                        # watch CPU, memory, and status
mena sessions                   # browse all saved sessions
mena ss                         # exact shorthand for `mena sessions`
mena inspect codex:session-id   # inspect one process or session
mena resume codex:session-id    # resume with the provider CLI
```

Targets are stable and script-friendly:

- Running process: `provider:PID`, for example `claude:43120`.
- Saved session: `provider:session-id`.
- Unqualified IDs work only when they resolve unambiguously.

## Current focus and roadmap

- **Now - Sessions:** active development focuses on `mena sessions` and its
  shorthand `mena ss`: discovery, search, transcript reading, exact response
  and tool metrics, export, resume, and safe deletion.
- **Next - Memory:** `mena memory` and durable agent-memory workflows are
  planned next. They are documented here for direction only and are **not
  implemented yet**.

## Sessions

`mena sessions` and `mena ss` are equivalent; every option works with either
form.

```sh
mena ss
mena ss --provider claude --limit 20
mena ss --plain
mena ss --json
```

The interactive session view can search across providers and projects, open the
complete transcript, show per-model persisted metrics, resume the native
session, export it as Markdown, or permanently delete it.

| Key | Action |
|---|---|
| `/` | Search target, provider, project, or title. Press `Enter` to also search the full transcript of every saved session (shown with a live progress indicator; `Esc` cancels) |
| `g` | Toggle list grouping — flat or grouped by project |
| `Enter` or `i` | Open session detail |
| `r` | Resume with the native provider CLI |
| `d`, then lowercase `y` | Permanently delete the selected session |
| `c` / `e` | Copy / export the current detail as Markdown (follows the preview scope below) |
| `Esc` or `q` | Close or quit |

Inside the detail view, use arrows or `j`/`k` to scroll, `PgUp`/`PgDn` to move
by page, `Home`/`End` to jump, and `Shift+↑`/`Shift+↓` to move between user and
assistant messages. The preview defaults to **conversation only** (user and
assistant messages); press `p` to keep just the conversation, or `Shift+P` to
reveal the complete transcript including tool calls, tool results, and system
messages. Copy (`c`) and export (`e`) follow the active preview scope; exports
are named `...-conv.md` (conversation) or `...-full.md` (complete). Metadata
and model usage are always shown. Mouse reporting remains disabled so native
terminal text selection still works.

## Commands

| Command | Purpose |
|---|---|
| `mena ps [--json]` | List recognized live processes |
| `mena top` | Open the live resource monitor |
| `mena sessions` / `mena ss` | Search and manage saved sessions |
| `mena inspect <target> [--json]` | Inspect a process or session |
| `mena logs <target> [-n N] [--raw]` | Read a bounded event tail |
| `mena resume [target]` | Pick or resume a native session |
| `mena stop <pid-target> [--force]` | Stop a revalidated process |
| `mena config init` | Create the private configuration file |

Use `mena <command> --help` for all options. Useful resume forms include:

```sh
mena resume               # interactive picker
mena resume --last        # most recently updated session
mena resume --list        # non-interactive candidate list
```

## Provider support

| Provider | Process discovery | Saved sessions | Resume | Delete |
|---|:---:|:---:|:---:|:---:|
| Claude Code | ✓ | ✓ | ✓ | ✓ |
| Codex | ✓ | ✓ | ✓ | ✓ |
| Gemini CLI | ✓ | ✓ | ✓ | ✓ |
| OpenCode | ✓ | ✓ | ✓ | ✓ |
| Pi | ✓ | ✓ | ✓ | ✓ |
| Oh My Pi | ✓ | ✓ | ✓ | ✓ |
| Cursor Agent | ✓ | - | ✓ | - |
| Custom recognizer | ✓ | - | configurable | - |

Cursor Agent and custom recognizers do not have a generic supported local
session catalog. `mena` returns an explicit unsupported error instead of
guessing a storage path.

### Live session association

`mena` reports live-session data only when current native evidence identifies
one logical session. Claude Code supplies PID, process-start, project, and
session identity metadata. Pi and Oh My Pi are matched only while the live
process holds exactly one cataloged native transcript open (via `/proc` on
Linux or `/usr/sbin/lsof` on macOS).

A provider resume/session argument proves only which session launched the
process; because an agent may switch sessions without restarting, it is shown
as `launch`, not `exact`. Project equality, timestamps, and “most recently
updated” are never used to claim a live association. `ambiguous`,
`unconfirmed`, and `unsupported` processes therefore expose no session ID,
tokens, or cost.

## Data and safety

- Conversation data stays in the provider's native local store.
- Tokens, cost, duration, TTFT, retries, and errors appear only when the native
  record contains them; missing values are never estimated.
- Live process metrics are attributed to a session only for an `exact` native
  association; `ps --json` exposes `session_match` and
  `session_match_evidence`.
- Resume commands use program-plus-argv execution and never invoke a shell.
- Stop revalidates PID, start time, executable, and provider before signaling.
- Delete rejects live sessions, ambiguous targets, path traversal, and symlink
  escapes outside provider-owned roots. If any live process is not exactly
  associated, deletion fails closed for that provider's complete catalog.
- Detail export is atomic, never overwrites an existing file, and uses mode
  `0600` on Unix.

## Automation

`ps`, `inspect`, and `sessions` expose stable JSON output:

```sh
mena ps --json | jq '.[] | {id, project, session_match, session_match_evidence, tokens, cost_usd}'
mena ss --json | jq '.[] | select(.agent == "codex") | .target'
```

A dash in human output means no exact native session was associated. `n/a`
means an exact session exists but does not contain a persisted cost.

## Configuration

Create `~/.config/mena/config.toml` with mode `0600` on Unix:

```sh
mena config init
```

The base directory honors `XDG_CONFIG_HOME`. Custom process recognizers use an
exact executable match and optional command markers:

```toml
[agent.custom.my_agent]
executables = ["my-agent"]
command_contains = ["--agent-mode"]
resume = ["my-agent", "resume", "{session}"]
```

The resume command is an argv array and must contain `{session}`. Existing
`clix` custom-agent definitions can be imported once with:

```sh
mena config init --import-clix
```

Normal operation reads only the `mena` configuration.

## Development

```sh
mise run verify  # fmt + check + tests + strict Clippy + rustdoc
mise run build   # optimized release binary
```

Provider adapters and safety-sensitive changes should include focused fixtures.
See [AGENTS.md](AGENTS.md) for architecture and repository invariants.

## License

[MIT](LICENSE) © 2026 sxwedo
