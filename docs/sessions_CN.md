# Session 管理

`mena sessions`（简写 `mena ss`）直接索引 Provider 自己保存的 Session，不会把
对话复制到 mena 数据库。

## 命令行

```sh
mena ss
mena ss --provider codex
mena ss --provider cursor --limit 20
mena ss --include-empty
mena ss --json
```

`--provider` 支持 `claude`、`codex`、`cursor`、`gemini`、`goose`、`opencode`、
`pi` 和 `omp`；`goose` 没有本地 Session 目录，始终列出为空。`--limit` 必须大于
等于 1。Cursor 中没有消息的草稿默认隐藏，传入 `--include-empty` 后才会显示。

已保存 Session 的 Target 格式是 `provider:session-id`。只有在结果唯一时，才允许
省略 Provider 或使用 ID 前缀。

## 交互式浏览器

详情按需加载，因此打开大目录时不会预先解析所有对话。

| 按键 | 操作 |
|---|---|
| `↑` / `↓`、`j` / `k` | 在可见行之间移动 |
| `PgUp` / `PgDn`、`Home` / `End` | 翻页或跳到边缘 |
| `/` | 搜索 Target、Provider、项目和标题 |
| 搜索时按 `Enter` | 启动可取消的全文对话搜索 |
| `g` | 切换平铺或按项目分组 |
| `Enter` / `i` | 打开选中的 Session |
| `r` | 使用 Provider 原生 CLI 恢复 |
| `R` | 选择另一个已安装的 Agent 继续 |
| `d`，再输入小写 `y` | 永久删除选中的 Session |
| `q` / `Esc` | 关闭或退出 |

详情视图按键：

| 按键 | 操作 |
|---|---|
| `↑` / `↓`、`j` / `k` | 滚动详情，不改变外层列表选择 |
| `PgUp` / `PgDn`、`Home` / `End` | 在对话内翻页或跳转 |
| `Shift+↑` / `Shift+↓` | 在用户与 Assistant 消息之间跳转 |
| `p` / `Shift+P` | 显示仅对话 / 完整记录 |
| `c` / `e` | 按当前预览范围复制 / 导出 Markdown |
| `r` | 恢复当前 Session |
| `R` | 选择另一个已安装的 Agent 继续 |
| `Esc` / `q` | 返回列表 |

仅对话导出以 `-conv.md` 结尾，完整导出以 `-full.md` 结尾；两种范围都会保留
元数据和按模型汇总的用量。

## 换 Agent 继续

大写 `R` 会保留小写 `r` 的原生恢复语义，并打开目标选择器。界面只展示已安装且
存在受支持路径的目标 Agent。

| 来源 Session | 目标 | 迁移方式 |
|---|---|---|
| Claude Code | Oh My Pi | 原生 `omp --from-claude` 导入 |
| Codex | Oh My Pi | 原生 `omp --from-codex` 导入 |
| 其他到 Claude、Codex 或 OMP 的跨 Agent 路径 | 所选目标 | handoff 后新建 Session |

OMP 的导入参数仍会打开 OMP 自己的来源选择器，因为其 CLI 不接受 mena 已选中的外部
Session ID 或路径。mena 会提示需要再次选择的来源 Target。OMP 会生成自己的新
Session；mena 绝不会把 Claude 或 Codex ID 传给 `omp --resume`。

handoff 会加载有界的完整对话，在记录的项目目录中写入临时 Markdown；Unix 下权限为
`0600`，目标进程退出后自动删除。目标获得的是持久化上下文，而不是实时运行状态：
Tool 结果、进程、权限、hook 和未提交工作区状态都必须重新核验。来源 Session 保持
不变。

## 持久化指标

根据 Provider 原生记录，详情可展示耗时、首 Token 延迟、Token 明细、成本、结束
原因、重试次数和结构化错误。工具调用可展示原生状态、耗时、退出码和错误。

缺失字段直接省略。mena 不会根据公开价格估算成本，也不会在 Provider 未持久化时
为单次 Tool 调用推算 Token。

## JSON 输出

`mena ss --json` 输出稳定的目录字段：

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

目录记录中不存在的字段不会被虚构出来。

## 安全与运行中 Session 关联

恢复命令始终以“程序 + argv”构造，不经过 Shell；启动前还会确认所选项目目录仍然
存在。
跨 Agent 启动同样不经过 Shell。handoff 文件保持私有、非持久化，也不会写入任何
Provider 的 Session 存储。

绿色 active 标记只接受精确的 Provider 原生证据：

- Claude Code 的运行时元数据必须同时匹配 PID、进程启动时间、项目和 Session。
- Pi 与 Oh My Pi 要求运行进程只打开一个已收录的原生对话文件（Linux 使用
  `/proc`，macOS 使用 `lsof`）。
- 恢复参数只能证明进程启动时的 Session，因为 Agent 可能不重启就切换 Session。

项目相同、时间接近或“最近更新”都不能建立精确关联。证据缺失或有歧义时，不会把
任何 Session 标成 active，但删除仍会对该 Provider 的目录执行 fail-closed 保护。

删除还会拒绝：

- 不在当前目录中的 Session；
- 不安全的 ID 或路径穿越；
- 规范路径或嵌套符号链接逃出 Provider 根目录；
- 被运行中 Agent 保护的 Session。

导出使用原子创建、不覆盖已有文件，并在 Unix 下使用 `0600` 权限。
