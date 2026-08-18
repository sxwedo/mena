# 记忆（Memories）

`mena memories`（别名 `mena ms`）发现各编码 Agent 在本地持久化的原生记忆与指令文件，
支持列出、读取、编辑和删除。发现过程是纯静态的：mena 绝不会为了收集记忆而启动任何进程。

## 命令行

```sh
mena ms
mena ms --provider claude
mena ms --scope project --json
mena ms inspect CLAUDE.md --json
mena ms inspect claude:user:CLAUDE.md
mena ms open codex:user:AGENTS.md
mena ms delete CLAUDE.local.md
```

Provider 过滤值为 `claude`、`codex`、`cursor`、`gemini`；scope 过滤值为
`user`、`project`。不支持的值会返回可操作的错误。

名称可以是裸文件名，也可以是 `provider:scope:name` 选择器。操作要求唯一匹配；
同名歧义会列出所有匹配的选择器，提示用 `--provider` 和/或 `--scope` 收窄。

## 发现位置

| Provider | 用户级 | 项目级 |
|---|---|---|
| Claude | `~/.claude/CLAUDE.md` | `CLAUDE.md`、`CLAUDE.local.md`，以及自动记忆目录 `~/.claude/projects/<编码后的 cwd>/memory/*.md` |
| Codex | `~/.codex/AGENTS.md`、`~/.codex/memories/*.md` | `AGENTS.md` |
| Cursor | — | `.cursor/rules/*.mdc` |
| Gemini | `~/.gemini/GEMINI.md` | — |

`AGENTS.md` 归属于 Codex（即使其他 Agent 也会读取它），因此只出现一次，不会被重复删除。

## 安全性

- 单文件读取上限为 1 MiB，超限直接失败。
- 拒绝读取符号链接记忆项。
- 每次读取或删除前都会重新校验解析后的规范路径仍位于扫描发现的
  Provider 所属根目录内。
- `open` 使用配置的编辑器（`VISUAL`、`EDITOR`，然后是常见回退项）；
  mena 自身从不改写文件内容。
- `delete` 需要显式的小写 `y` 确认，且只删除普通文件。
