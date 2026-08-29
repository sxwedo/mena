# Sessions

`mena sessions` (alias `mena ss`) catalogs provider-owned sessions without
copying them into a mena database.

## Command line

```sh
mena ss
mena ss --provider codex
mena ss --provider cursor --limit 20
mena ss --include-empty
mena ss --json
```

`--provider` accepts `claude`, `codex`, `cursor`, `gemini`, `goose`, `opencode`,
`pi`, or `omp`; `goose` has no native local session catalog and always lists
nothing. `--limit` must be at least 1. Cursor draft sessions without messages
are hidden unless `--include-empty` is present.

A saved-session target is `provider:session-id`. An unqualified ID or prefix is
accepted only when it resolves to one logical session.

## Interactive browser

The browser reads detail lazily, so opening a large catalog does not parse every
transcript up front.

| Key | Action |
|---|---|
| `↑` / `↓`, `j` / `k` | Move through visible rows |
| `PgUp` / `PgDn`, `Home` / `End` | Move by page or jump to an edge |
| `/` | Search target, provider, project, and title |
| `Enter` while searching | Run a cancellable full-transcript search |
| `g` | Toggle flat or project-grouped rows |
| `Space` | Toggle a delete mark on the selected row |
| `a` | Mark or unmark every visible session |
| `Enter` / `i` | Open the selected session |
| `r` | Resume with the provider's native CLI |
| `R` | Hand off to another installed agent |
| `d`, then lowercase `y` | Permanently delete the selected session, or every marked session |
| `q` / `Esc` | Quit (`Esc` first clears marks when any exist) |

With marks present the browser enters batch mode: the footer shows only the
delete actions, single-session keys (`r`, `R`, `Enter`, `g`, `/`) explain that
they are locked until `Esc` clears the marks, and the confirmation dialog lists
up to five targets with imperative key guidance (`y` deletes, `n`/`Esc` keeps
everything). Sessions protected by a running agent are skipped from the batch
with a notice instead of blocking the whole deletion.

The list shows compact targets (`provider:first-8-id-characters`) to leave
room for titles; the detail view, delete confirmations, exports, clipboard
content, and `--json` output always carry the full `provider:session-id`.

Inside the detail view:

| Key | Action |
|---|---|
| `↑` / `↓`, `j` / `k` | Scroll without moving the outer selection |
| `PgUp` / `PgDn`, `Home` / `End` | Page or jump within the transcript |
| `Shift+↑` / `Shift+↓` | Jump between user and assistant messages |
| `p` / `Shift+P` | Show conversation-only / complete transcript |
| `/` | Search the transcript; `Enter` keeps, `Esc` cancels |
| `n` / `N` | Jump to the next / previous match after a search |
| `c` / `e` | Copy / export Markdown using the active preview scope |
| `r` | Resume this session |
| `R` | Hand off to another installed agent |
| `Esc` / `q` | Return to the list |

Conversation-only exports end in `-conv.md`; complete exports end in
`-full.md`. Metadata and per-model usage remain present in both scopes.

## Continue with another agent

Uppercase `R` keeps lowercase `r` native and opens a target selector. Only
installed targets with a supported route are shown.

| Source session | Target | Transfer |
|---|---|---|
| Claude Code | Oh My Pi | Native `omp --from-claude` importer |
| Codex | Oh My Pi | Native `omp --from-codex` importer |
| Any other cross-agent route to Claude, Codex, or OMP | Selected target | Handoff into a fresh session |

OMP's import flags open OMP's own source picker because its CLI does not accept
the already-selected foreign session ID or path. mena displays the source
target to select. OMP creates its own new session; mena never passes a Claude or
Codex ID to `omp --resume`.

A handoff loads the complete bounded transcript, writes a temporary Markdown
file inside the recorded project with private `0600` permissions on Unix, and
starts the target with a prompt pointing to that file. The file is removed when
the target process exits. The target receives persisted context, not live
runtime state: tool results, processes, permissions, hooks, and uncommitted
workspace state must be verified again. The source session remains unchanged.

## Persisted metrics

Depending on the provider record, detail can show duration, time to first token,
token breakdown, cost, finish reason, retry count, and structured errors.
Tool calls can show their native status, duration, exit code, and error.

Missing fields are omitted. In particular, mena does not estimate token cost
from public prices and does not assign a per-call Tool token value when the
provider did not persist one.

## JSON output

`mena ss --json` emits the catalog shape used by automation:

```json
{
  "target": "codex:019f...",
  "agent": "codex",
  "session_id": "019f...",
  "title": "Refactor session catalog",
  "project": "/work/mena",
  "log": "/home/me/.codex/sessions/...jsonl",
  "started_at": "2026-08-12T08:00:00Z",
  "updated_at_unix": 1786500000
}
```

The command does not claim fields that are absent from the catalog record.

## Safety and live-session association

Resume commands are constructed as a program plus argv and never passed to a
shell. The selected project must still exist before mena starts the provider.
Cross-agent launch commands follow the same shell-free rule. Handoff files are
private, non-persistent, and never written into a provider's session store.

An active indicator requires exact provider-native evidence:

- Claude Code runtime metadata must agree on PID, process start, project, and
  session identity.
- Pi and Oh My Pi require the live process to hold exactly one cataloged native
  transcript open (`/proc` on Linux or `lsof` on macOS).
- A resume argument is launch evidence only because an agent may switch
  sessions without restarting.

Project equality, recency, and timestamps alone never establish an exact
association. If evidence is missing or ambiguous, no session is marked active,
but deletion still fails closed for that provider's catalog.

Deletion also rejects:

- a session absent from the current catalog;
- unsafe IDs or path traversal;
- canonical paths or nested symlinks escaping provider-owned roots;
- a session protected by a running agent. Batch deletion skips protected
  sessions and reports them instead of aborting the batch; a protected
  session is never deleted.

Exports are created atomically, never overwrite an existing file, and use mode
`0600` on Unix.
