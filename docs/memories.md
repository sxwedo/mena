# Memories

`mena memories` (alias `mena ms`) discovers the native memory and instruction
files coding agents persist locally, and can list, read, edit, or delete them.
Discovery is static: mena never launches a process to collect memories.

## Command line

```sh
mena ms
mena ms --provider claude
mena ms --scope project --json
mena ms --list
mena ms inspect CLAUDE.md --json
mena ms inspect claude:user:CLAUDE.md
mena ms open codex:user:AGENTS.md
mena ms delete CLAUDE.local.md
```

Provider filters are `claude`, `codex`, `cursor`, `gemini`, `goose`, `omp`, and
`pi`; scope filters are `user` and `project`. Unsupported values fail with an
actionable error.

Names may be bare file names or `provider:scope:name` selectors. An operation
requires one unique match; ambiguous names fail with the matching selectors
listed so they can be narrowed with `--provider` and/or `--scope`.

## Interactive browser

With an interactive terminal, `mena ms` opens a master-detail browser using the
same Calm Console shell as the other surfaces. `--list` prints the plain table
instead.

| Key | Action |
|---|---|
| `↑` / `↓`, `j` / `k` | Move through files, or scroll the preview after `Tab` |
| `Home` / `End`, `PgUp` / `PgDn` | Jump or page the preview |
| `Tab` / `l`, `h` | Switch between list and preview |
| `/` | Filter by name, provider, scope, or location |
| `o` | Edit the selected file in `$VISUAL`/`$EDITOR` |
| `d`, then lowercase `y` | Delete the selected file after confirmation |
| `?` | Show the full keymap overlay |
| `1` / `2` / `3` | Jump to Sessions / Skills / MCP |
| `q` / `Esc` | Quit |

The confirmation dialog names the exact file, selector, and path before the
lowercase `y` takes effect.

## Discovery locations

| Provider | User | Project |
|---|---|---|
| Claude | `~/.claude/CLAUDE.md` | `CLAUDE.md`, `CLAUDE.local.md`, and the auto-memory directory `~/.claude/projects/<encoded-cwd>/memory/*.md` |
| Codex | `~/.codex/AGENTS.md`, `~/.codex/AGENTS.override.md`, `~/.codex/memories/*.md` | `AGENTS.md`, `AGENTS.override.md` |
| Cursor | — | `.cursor/rules/*.mdc` |
| Gemini | `~/.gemini/GEMINI.md` | — |
| Goose | `~/.config/goose/.goosehints` | `.goosehints` |
| OMP | `~/.omp/agent/AGENTS.md` | — |
| Pi | `~/.pi/agent/AGENTS.md` | — |

`AGENTS.md` is attributed to Codex even when other agents also read it, so it
appears once and cannot be deleted twice. Claude auto-memory is omitted when
the workspace path contains `-`, because Claude encodes both `-` and `/` as
`-` and two projects can share one directory.

## Safety

- Reads are bounded to 1 MiB per file; larger files fail closed.
- Symlinked memory entries are refused.
- Every read or deletion re-validates that the resolved canonical path stays
  inside a provider-owned root discovered during the scan.
- `open` uses the configured editor (`VISUAL`, `EDITOR`, then common
  fallbacks); mena never rewrites the file itself.
- `delete` requires an explicit lowercase `y` confirmation and only removes
  regular files.
