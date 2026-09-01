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

`mena` is a local-first CLI for launching coding agents, browsing native saved
sessions, inspecting Agent Skills, and auditing MCP registrations. It runs
without a daemon, account, or remote data store.

## Install

Requires Rust 1.96.1 or newer; `cargo install` builds from source.

```sh
cargo install --git https://github.com/sxwedo/mena --locked
mena --version
```

To build from a checkout:

```sh
git clone https://github.com/sxwedo/mena.git
cd mena
mise run build
```

## Quick start

```sh
mena agent                    # choose and launch an agent in the current directory
mena ag claude                # launch Claude Code
mena ag codex --resume        # resume the latest Codex session in this project
mena ps                       # list running coding-agent processes
mena sessions                 # browse saved sessions
mena ss --provider cursor     # filter sessions by provider
mena skills                   # browse installed Agent Skills
mena sk inspect ponytail      # inspect one uniquely named Skill
mena mcp                      # interactively browse MCP registrations
mena mcp open codegraph       # open source config at the registration
mena mcp inspect codegraph --probe  # explicitly discover live MCP metadata
mena memories                 # list agent memory files
mena ms inspect CLAUDE.md     # read one uniquely identified memory file
```

| Command | Purpose |
|---|---|
| `mena agent` / `mena ag` | Launch an agent, fresh or from a native session |
| `mena ps` | List running coding-agent processes |
| `mena sessions` / `mena ss` | Search, inspect, resume, continue with another agent, export, and delete sessions |
| `mena skills` / `mena sk` | List, filter, inspect, and browse Agent Skills |
| `mena mcp` | Browse grouped registrations; open/edit/delete config; explicitly probe metadata |
| `mena memories` / `mena ms` | List, read, edit, and delete agent memory files |
| `mena config init` | Create `~/.config/mena/config.toml` |

## Provider support

| Provider | Launch | Sessions | Continue with | MCP config |
|---|:---:|:---:|---|:---:|
| Claude Code | ✓ | ✓ | OMP import; Codex handoff | ✓ |
| Codex | ✓ | ✓ | OMP import; Claude handoff | ✓ |
| Gemini CLI | ✓ | ✓ | Claude/Codex/OMP handoff | ✓ |
| OpenCode | ✓ | ✓ | Claude/Codex/OMP handoff | ✓ |
| Pi | ✓ | ✓ | Claude/Codex/OMP handoff | adapter¹ |
| Oh My Pi | ✓ | ✓ | Claude/Codex handoff | ✓ |
| Cursor Agent | ✓ | ✓ | Claude/Codex/OMP handoff | ✓ |
| Grok | ✓ | ✓ | Claude/Codex/OMP handoff | — |
| Goose | ✓ | — | — | ✓ |
| Custom configuration | ✓ | — | — | — |

¹ Pi entries are discovered only when `pi-mcp-adapter` is installed.

Custom agents and Goose do not have a generic session catalog. `mena` returns
an explicit unsupported result instead of guessing provider-owned paths.
Inside the session browser, `r` performs native resume and `R` chooses an
installed target agent. See the session documentation for import and handoff
semantics.

## Documentation

- [Calm Console interface and responsive behavior](docs/interface.md)
- [Session browser, metrics, export, and deletion](docs/sessions.md)
- [Agent Skill discovery and browser](docs/skills.md)
- [MCP catalog, metadata, sources, and safety](docs/mcp.md)
- [Agent memory files](docs/memories.md)
- [Configuration](docs/configuration.md)
- [Architecture and development](docs/development.md)

## Safety model

- Session data remains in each provider's native local store.
- Usage and cost are shown only when persisted by the provider; values are
  never inferred from public prices.
- Resume uses program-plus-argv execution, never a shell.
- Cross-agent handoffs use a temporary private Markdown file and start a fresh
  target session; provider runtime state is never presented as transferred.
- `mena ps` is a one-shot, read-only OS process snapshot. Its status is not an
  inferred agent lifecycle; `--verbose` prints full command lines that may contain secrets.
- The MCP browser starts from static configuration. Only `p` or `--probe`
  starts stdio or contacts HTTP; probes never call tools or read resources.
- Live-session association requires provider-native evidence. Uncertain cases
  fail closed for deletion.
- Session deletion validates identifiers, canonical paths, symlink containment,
  and live-process protection before removing data.
- Memory discovery is static and read-bounded; edits go through the configured
  editor and deletions require explicit confirmation with root containment.

See [the session safety details](docs/sessions.md#safety-and-live-session-association)
for the complete contract.

## Development

```sh
mise run verify  # fmt + check + tests + strict Clippy + rustdoc
mise run build   # optimized release binary
```

Repository architecture and extension points are documented in
[docs/development.md](docs/development.md). Agent-specific invariants live in
[AGENTS.md](AGENTS.md).

## License

[MIT](LICENSE) © 2026 sxwedo
