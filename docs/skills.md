# Agent Skills

`mena skills` (alias `mena sk`) discovers Skill entrypoints in standard global
and workspace directories, parses their frontmatter, and provides a terminal
tree browser for related files.

## Command line

```sh
mena sk
mena sk --provider codex
mena sk --scope workspace --json
mena sk inspect ponytail
mena sk --provider codex --scope global inspect ponytail --json
```

Provider filters are `claude`, `codex`, `cursor`, `opencode`, and `omp`; scope
filters are `global` and `workspace`. Unsupported values fail with an
actionable error.

`inspect` requires one unique match. If the same name exists in multiple
providers or scopes, use `--provider` and/or `--scope` instead of accepting an
arbitrary first result.

## Discovery locations

| Label | Global | Workspace |
|---|---|---|
| Claude | `~/.claude/skills` | `.claude/skills` |
| Codex | `~/.codex/skills` | `.codex/skills` |
| Cursor | `~/.cursor/rules` | `.cursor/rules` |
| OpenCode | `~/.config/opencode/skills` | `.opencode/skills` |
| OMP/shared | `~/.agents/skills` | `.agents/skills` |

An entry may be a direct Markdown file or a directory containing `SKILL.md`,
`skill.md`, or `README.md` (checked in that order).

## Interactive browser

| Key | Action |
|---|---|
| `↑` / `↓`, `j` / `k` | Move through the tree or scroll the focused preview |
| `Space` / `→` / `Enter` | Expand or collapse the selected directory |
| `←` / `h` | Collapse the current directory or its nearest ancestor |
| `Tab` / `l` | Switch between tree and preview |
| `PgUp` / `PgDn` | Scroll the preview |
| `/` | Filter by name, provider, scope, location, trigger, or description |
| `s` | Show or hide top-level symlinked Skills |
| `o` | Open the selected file's containing directory |
| `q` / `Esc` | Close the preview or quit |

Directory contents are loaded through the Skill catalog and cached for the
browser. Search and redraw do not repeatedly walk the filesystem.

## Read limits and containment

- Preview reads are limited to 8 MiB per file.
- A directory is limited to 10,000 entries.
- Files must be valid UTF-8 for text preview.
- Canonical paths must remain inside the selected Skill root. A symlinked Skill
  root is supported, but a nested symlink cannot escape that resolved root.

These limits affect inspection and preview only; mena never edits Skill files.
