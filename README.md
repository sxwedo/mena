<p align="center">
  <img src="https://github.com/user-attachments/assets/b5d7500e-4d16-4e5f-a414-872718010a22" width="220" alt="mena logo">
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-2563eb.svg" alt="License: MIT"></a>
  <a href=".mise.toml"><img src="https://img.shields.io/badge/rust-1.96.1-f97316.svg" alt="Rust 1.96.1"></a>
  <img src="https://img.shields.io/badge/data-local--first-10b981.svg" alt="Local first">
</p>

<p align="center"><strong>One fast, local control plane for your developer agents.</strong></p>

<p align="center">
  English | <a href="README_CN.md">简体中文</a>
</p>

<code>mena</code> discovers running coding agents, connects them to their native
persisted sessions, shows exact recorded usage, and gives you a safe way to
inspect, resume, stop, export, or delete them. There is no daemon, account, or
remote service.

- **One view across providers.** Claude Code, Codex, Gemini CLI, OpenCode, Pi,
  Oh My Pi, Cursor Agent, and custom process recognizers.
- **Live and historical context.** CPU, memory, project, session, transcript,
  persisted tokens, recorded cost, model, and per-response metrics.
- **Native by design.** Reads provider-owned stores in place and resumes through
  native argv without a shell.
- **Private by default.** Conversation data stays on the machine; logs are
  bounded and redacted unless raw output is explicitly requested.
- **Safety at destructive boundaries.** Process identity is revalidated before
  signaling, and session deletion fails closed on live, escaped, or ambiguous
  targets.

## Install

### From Git

~~~sh
cargo install --git https://github.com/sxwedo/mena --locked
~~~

### From source

~~~sh
git clone https://github.com/sxwedo/mena.git
cd mena
mise run build
install -m 0755 target/release/mena /usr/local/bin/mena
~~~

The repository pins Rust 1.96.1 in <code>.mise.toml</code>. The native provider
CLIs and session stores are optional: mena exposes only the providers present on
the current machine.

## 60-second quick start

~~~sh
# See every recognized live agent.
mena ps

# Watch resource use in a responsive terminal UI.
mena top

# Search all saved native sessions, including inactive ones.
mena sessions

# Inspect or resume with a stable target.
mena inspect codex:019abcde-session-id
mena resume codex:019abcde-session-id
~~~

Targets are stable and script-friendly:

- A running process uses <code>provider:PID</code>, such as
  <code>claude:43120</code>.
- A persisted session uses <code>provider:session-id</code>.
- An unqualified ID is accepted only when it resolves unambiguously.

## Core workflows

### Monitor live agents

~~~sh
mena ps
mena ps --json

mena top
mena top --interval 3 --iterations 5
~~~

The process view reports <code>ID</code>, <code>AGENT</code>,
<code>PROJECT</code>, <code>STATUS</code>, <code>DURATION</code>,
<code>TOKENS</code>, and <code>COST</code>. The live TUI adds CPU and memory.

| Key | Action |
|---|---|
| <code>↑</code>/<code>↓</code> or <code>j</code>/<code>k</code> | Select a process |
| <code>Enter</code> or <code>i</code> | Open details |
| <code>r</code> | Refresh |
| <code>q</code> | Quit |

### Find and inspect saved sessions

~~~sh
mena sessions
mena sessions --provider claude --limit 20
mena sessions --plain
mena sessions --json

mena inspect codex:019abcde-session-id
mena logs claude:session-id -n 100
mena logs claude:session-id -n 20 --raw
~~~

<code>mena sessions</code> keeps <code>TARGET</code> as the first and widest
fixed column so the complete selector remains visible. Wider terminals add
agent, project, title/summary, and updated time; narrow layouts retain target and
title.

| Session list key | Action |
|---|---|
| <code>/</code> | Search target, provider, project, or title |
| <code>Enter</code> or <code>i</code> | Open the complete detail |
| <code>r</code> | Resume through the provider CLI |
| <code>d</code>, then lowercase <code>y</code> | Permanently delete |
| <code>Esc</code> or <code>q</code> | Close or quit |

The detail popup preserves every parsed message in provider order. It does not
truncate long content, and its scroll range uses the actual wrapped height
instead of a 16-bit row limit.

| Detail key | Action |
|---|---|
| <code>↑</code>/<code>↓</code> or <code>j</code>/<code>k</code> | Scroll |
| <code>Shift+↑</code>/<code>Shift+↓</code> | Jump between user and assistant messages |
| <code>PgUp</code>/<code>PgDn</code> | Move by page |
| <code>Home</code>/<code>End</code> | Jump to the beginning or end |
| <code>c</code> | Copy the complete detail as Markdown |
| <code>e</code> | Export the complete detail as Markdown |
| <code>r</code> | Resume the session |
| <code>Enter</code> or <code>Esc</code> | Return without losing list selection |

Mouse reporting remains disabled, so native terminal drag selection and
<code>Command+C</code> continue to work. Alternate-scroll mode maps touchpad and
wheel gestures to navigation while leaving clicks and drags to the terminal.
Same-direction inertial events are coalesced to prevent a queued scroll from
continuing after input stops.

### Resume and control

~~~sh
# Pick interactively, resume the newest, or list candidates.
mena resume
mena resume --last
mena resume --list --limit 20

# Resume directly.
mena resume codex:session-id
mena resume omp:session-id
mena resume cursor:chat-id

# Graceful termination after identity revalidation; --force kills.
mena stop claude:43120
mena stop claude:43120 --force
~~~

Resume commands are constructed as program plus argv. mena never invokes a
shell or interpolates arbitrary command text.

## Session detail

### Message styling

The default palette keeps the transcript scannable without turning it into a
rainbow:

| Category | Default |
|---|---|
| User header and content | Light green |
| Assistant header and content | Cyan |
| Skill header and content | Light yellow |
| Tool call, tool result, system/meta, and error | Muted dark gray |
| Metadata keys | Light magenta |

Every header and content color can be overridden independently. See
[Configuration](#configuration).

### Response, model, and token metrics

When the provider persists a model ID, it appears on the response that produced
the assistant text or tool call. Exact duration, total tokens, and cost follow
it when present:

~~~text
[2026-07-30T13:42:04.795Z] ASSISTANT · gpt-5.6 · 12.3s · 67,890 tokens · $0.1250
Tokens: input 50,000 · output 10,000 · cache read 7,000 · cache write 500 (5m 400 · 1h 100) · reasoning 390 · tool 1,000
Response: status completed · stop reason stop · TTFT 450ms · retries 1
~~~

The token line shows only fields in the provider's native record. This includes
Claude's 5-minute and 1-hour cache-creation split and Gemini's request-level
tool token count when available. Missing fields are omitted rather than guessed.
A provider may count cached input inside input, so mena preserves an explicit
provider total instead of recomputing it.

The response line normalizes a readable status from the persisted stop reason
and may include TTFT, retry count, and native error code/message. Cost is shown
per response only when that exact value is persisted; mena never estimates it
from a public price table. A model-usage section above the conversation sums the
available response count, duration, average TTFT, tokens, cost, retries, and
errors for each model. Missing samples do not become zeros.

Tool calls show their persisted status, duration, exit code, and error details:

~~~text
[2026-07-30T13:42:05.100Z] TOOL CALL · completed · 140ms · exit 0
Token accounting: provider response totals only; no per-call token value is persisted.
~~~

Provider stores currently do not expose a trustworthy token count for each
individual tool execution. mena therefore keeps Gemini's exact tool-token value
on the model response and never divides or attributes it across tool calls.

### Markdown export

Press <code>e</code> in detail view. The export contains complete metadata,
ordered messages, a per-model summary, model IDs, exact response and tool
metrics, and structured tool content. Token breakdowns use the same
available-field rules as the detail popup, so copying with <code>c</code> and
exporting with <code>e</code> produce equivalent metric content.

- Destination: the directory where mena was started.
- Name:
  <code>mena-session-{provider}-{safe-id}-{YYYYMMDD-HHMMSS}.md</code>.
- Collision handling: <code>-2</code>, <code>-3</code>, and so on; existing
  files are never replaced.
- Durability: atomic creation with no partial file on failure.
- Privacy: mode <code>0600</code> on Unix. Other platforms retain atomic,
  no-overwrite behavior without promising Unix permission bits.

The popup stays open after copy or export and shows either the absolute result
path or an actionable error without moving the selection or scroll position.

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

Cursor Agent and custom recognizers have no generic supported local session
catalog. Archived-session operations return an explicit unsupported error
instead of guessing a storage path.

Persisted detail fields vary by Provider:

| Provider | Exact response detail | Exact tool detail |
|---|---|---|
| Claude Code | Input/output, cache read/write and 5m/1h cache creation, stop reason | Result status/error and elapsed time from persisted timestamps |
| Codex | Last-request tokens, turn duration, TTFT, completion/interruption | End-event status, duration, and error when present |
| Gemini CLI | Input/output/cached/thought/tool tokens, duration and stop/error fields when present | Call status and result; no per-call token value |
| OpenCode | Tokens, cost, duration, TTFT, finish reason, retry/error fields | Status, duration, exit code, and error |
| Pi | Tokens, cost, stop/error fields when present | Result status/error and persisted elapsed time when available |
| Oh My Pi | Tokens, cost, duration, TTFT, stop/retry/error fields | Status, duration, exit code, and error when present |

Every cell is availability-based: mena omits a value when that session record
does not contain it.

## Command reference

| Command | Purpose |
|---|---|
| <code>mena ps [--json]</code> | List recognized live processes |
| <code>mena top</code> | Watch CPU, memory, status, and exact persisted usage |
| <code>mena inspect &lt;target&gt; [--json]</code> | Inspect a live process or saved session |
| <code>mena logs &lt;target&gt; [-n N] [--raw]</code> | Read a bounded event tail |
| <code>mena sessions</code> | Search, inspect, resume, export, or delete sessions |
| <code>mena stop &lt;pid-target&gt; [--force]</code> | Stop after process identity revalidation |
| <code>mena resume [session-target]</code> | Resume through the native provider CLI |
| <code>mena config init</code> | Create a private configuration file |

Use <code>mena &lt;command&gt; --help</code> for every option.

### Automation output

<code>mena ps --json</code>, <code>mena inspect --json</code>, and
<code>mena sessions --json</code> emit stable machine-readable fields.

~~~sh
mena ps --json | jq '.[] | {id, project, tokens, cost_usd}'
mena sessions --json | jq '.[] | select(.agent == "codex") | .target'
~~~

A dash in human-readable process output means no native session was associated.
<code>n/a</code> means a session exists but does not contain an exact recorded
cost.

## Configuration

Create <code>~/.config/mena/config.toml</code> with private permissions:

~~~sh
mena config init
~~~

The file uses mode <code>0600</code> on Unix. Its base directory honors
<code>XDG_CONFIG_HOME</code>.

### Custom process recognizers

~~~toml
[agent.custom.my_agent]
executables = ["my-agent", "my-agent.exe"]
command_contains = ["--agent-mode"]
resume = ["my-agent", "resume", "{session}"]
~~~

Executable basenames are matched exactly. Every configured resume command is an
argv array and must contain the <code>{session}</code> placeholder.

### Detail colors

All keys are optional; omitted keys keep the defaults.

~~~toml
[ui.session_detail.colors]
border = "cyan"
popup_title = "reset"
metadata_key = "light-magenta"
metadata_value = "reset"
conversation_header = "cyan"
empty_text = "dark-gray"
status_success = "green"
status_error = "red"
footer_key = "cyan"
footer_text = "reset"
footer_separator = "dark-gray"

user_header = "light-green"
user_content = "light-green"
assistant_header = "cyan"
assistant_content = "cyan"
skill_header = "light-yellow"
skill_content = "light-yellow"
tool_call_header = "dark-gray"
tool_call_content = "dark-gray"
tool_result_header = "dark-gray"
tool_result_content = "dark-gray"
system_header = "dark-gray"
system_content = "dark-gray"
error_header = "dark-gray"
error_content = "dark-gray"
~~~

Accepted values are <code>reset</code>, standard ANSI names, an indexed color
such as <code>ansi:45</code>, or true color such as <code>#a1b2c3</code>.
Invalid values fail at startup with the exact config path instead of silently
falling back. Restart mena after editing the file.

## Performance and safety model

- Catalog discovery retains only the metadata needed to identify sessions;
  usage and complete transcripts stay lazy.
- Exact usage is loaded lazily and cached until the backing file changes.
- Opening a detail performs one provider-native traversal that produces both
  aggregate usage and the complete ordered transcript. It does not reread the
  session solely for totals.
- JSON documents, individual JSONL records, log tails, and catalog scans have
  explicit bounds.
- Secrets in common process arguments are redacted in default output.
  <code>--raw</code> is an explicit opt-in to original records.
- Stop revalidates PID, start time, executable, and provider immediately before
  signaling.
- Delete validates storage IDs and canonical roots, rejects symlink or traversal
  escapes, removes known provider sidecars/indexes, and refuses sessions
  attached to a running process.
- Destructive confirmation accepts lowercase <code>y</code> only.

## Troubleshooting

**No saved sessions appear for Cursor Agent or a custom recognizer.** Their
processes can be discovered and resumed, but mena deliberately does not guess a
native archive path.

**Tokens or cost are missing.** mena reports only exact values present in the
native record. It does not infer usage or price.

**Terminal drag selection does not copy.** Use the terminal's normal selection
gesture and platform copy shortcut. Press <code>c</code> when you want the
entire detail serialized to Markdown in one operation.

**A transcript cannot be opened completely.** A native JSONL record exceeded
the safety bound. mena fails with the affected path rather than silently showing
a partial conversation.

**Resume fails.** Confirm that the selected provider CLI is installed and
available on <code>PATH</code>, then retry with the full provider-qualified
target.

## Architecture

~~~text
src/
├── main.rs           CLI entrypoint and exit handling
├── lib.rs            command definitions and dispatch
├── controller.rs     orchestration, target resolution, JSON, native resume
├── process.rs        discovery, recognition, resource sampling, safe stop
├── session.rs        catalog discovery, bounded I/O, deletion safety
├── session/
│   └── detail.rs     single-pass provider usage and transcript adapters
├── export.rs         complete Markdown rendering and collision-safe export
├── tui.rs            responsive process and session interfaces
├── view.rs           stable plain-text tables and formatting
├── settings.rs       private mena configuration and UI preferences
├── fs.rs             atomic replacement and private no-overwrite creation
└── ui.rs             terminal status and error presentation
~~~

The public modules stay deliberately small. Provider-specific persisted schemas
are hidden behind the session detail adapter; filesystem mutation remains
centralized behind validated deletion and atomic-write interfaces.

## Development and contributing

~~~sh
mise run verify  # fmt + check + tests + strict Clippy + rustdoc
mise run build   # optimized release binary
~~~

Changes should preserve stable selectors and JSON fields, include provider
fixtures for native-store behavior, and keep destructive operations fail-closed.
Before opening a pull request:

1. Add or update focused tests.
2. Run <code>mise run verify</code>.
3. Run <code>mise run build</code> for release-impacting changes.
4. Update this README when user-facing behavior changes.

Issues and focused pull requests are welcome. For security-sensitive findings,
avoid posting native session content, process arguments, or filesystem paths in
public reports.

## License

[MIT](LICENSE) © 2026 sxwedo
