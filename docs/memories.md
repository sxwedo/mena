# Memories

`mena memories` (alias `mena ms`) discovers the native memory and instruction
files coding agents persist locally, and can list, read, edit, or delete them.
Discovery is static: mena never launches a process to collect memories.

## Command line

```sh
mena ms
mena ms --provider claude
mena ms --scope project --json
mena ms inspect CLAUDE.md --json
mena ms inspect claude:user:CLAUDE.md
mena ms open codex:user:AGENTS.md
mena ms delete CLAUDE.local.md
```

Provider filters are `claude`, `codex`, `cursor`, and `gemini`; scope filters
are `user` and `project`. Unsupported values fail with an actionable error.

Names may be bare file names or `provider:scope:name` selectors. An operation
requires one unique match; ambiguous names fail with the matching selectors
listed so they can be narrowed with `--provider` and/or `--scope`.

## Discovery locations

| Provider | User | Project |
|---|---|---|
| Claude | `~/.claude/CLAUDE.md` | `CLAUDE.md`, `CLAUDE.local.md`, and the auto-memory directory `~/.claude/projects/<encoded-cwd>/memory/*.md` |
| Codex | `~/.codex/AGENTS.md`, `~/.codex/memories/*.md` | `AGENTS.md` |
| Cursor | — | `.cursor/rules/*.mdc` |
| Gemini | `~/.gemini/GEMINI.md` | — |

`AGENTS.md` is attributed to Codex even when other agents also read it, so it
appears once and cannot be deleted twice.

## Safety

- Reads are bounded to 1 MiB per file; larger files fail closed.
- Symlinked memory entries are refused.
- Every read or deletion re-validates that the resolved canonical path stays
  inside a provider-owned root discovered during the scan.
- `open` uses the configured editor (`VISUAL`, `EDITOR`, then common
  fallbacks); mena never rewrites the file itself.
- `delete` requires an explicit lowercase `y` confirmation and only removes
  regular files.
