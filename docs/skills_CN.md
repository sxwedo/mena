# Agent Skill

`mena skills`（简写 `mena sk`）会在标准的全局和工作区目录中发现 Skill 入口，
解析 frontmatter，并通过终端目录树浏览相关文件。

## 命令行

```sh
mena sk
mena sk --provider codex
mena sk --scope workspace --json
mena sk inspect ponytail
mena sk --provider codex --scope global inspect ponytail --json
```

Provider 筛选支持 `claude`、`codex`、`cursor`、`opencode` 和 `omp`；Scope 支持
`global` 与 `workspace`。不支持的值会返回明确错误。

`inspect` 必须唯一命中。如果同名 Skill 存在于多个 Provider 或 Scope，必须使用
`--provider` 和/或 `--scope` 缩小范围，不会静默选择第一条。

## 发现位置

| 标识 | 全局 | 工作区 |
|---|---|---|
| Claude | `~/.claude/skills` | `.claude/skills` |
| Codex | `~/.codex/skills` | `.codex/skills` |
| Cursor | `~/.cursor/rules` | `.cursor/rules` |
| OpenCode | `~/.config/opencode/skills` | `.opencode/skills` |
| OMP / 共享 | `~/.agents/skills` | `.agents/skills` |

入口可以是直接放置的 Markdown 文件，也可以是包含 `SKILL.md`、`skill.md` 或
`README.md` 的目录；目录入口按这个顺序查找。

## 交互式浏览器

| 按键 | 操作 |
|---|---|
| `↑` / `↓`、`j` / `k` | 在目录树移动，或滚动当前聚焦的预览 |
| `Space` / `→` / `Enter` | 展开或折叠选中的目录 |
| `←` / `h` | 折叠当前目录或最近的上级目录 |
| `Tab` / `l` | 在目录树和预览之间切换焦点 |
| `PgUp` / `PgDn` | 滚动预览 |
| `/` | 按名称、Provider、Scope、位置、Trigger 或描述筛选 |
| `s` | 显示或隐藏顶层符号链接 Skill |
| `o` | 打开所选文件所在目录 |
| `q` / `Esc` | 关闭预览或退出 |

目录内容统一通过 Skill Catalog 加载并缓存；搜索和界面重绘不会反复遍历文件系统。

## 读取限制与路径范围

- 单个预览文件最多读取 8 MiB。
- 单个目录最多包含 10,000 个条目。
- 文本预览要求文件是合法 UTF-8。
- 规范路径必须位于所选 Skill 根目录内。顶层 Skill 可以是符号链接，但嵌套符号
  链接不能逃出解析后的根目录。

这些限制只影响检查和预览；mena 不会修改 Skill 文件。
