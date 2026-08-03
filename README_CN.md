<p align="center">
  <img src="https://github.com/user-attachments/assets/b5d7500e-4d16-4e5f-a414-872718010a22" width="220" alt="mena 标志">
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-2563eb.svg" alt="MIT 许可证"></a>
  <a href=".mise.toml"><img src="https://img.shields.io/badge/rust-1.96.1-f97316.svg" alt="Rust 1.96.1"></a>
  <img src="https://img.shields.io/badge/data-local--first-10b981.svg" alt="本地优先">
</p>

<p align="center"><strong>智能协作，有迹可循；未完之章，从容再续。</strong></p>

<p align="center">
  <a href="README.md">English</a> | 简体中文
</p>

`mena` 可以发现正在运行的编码 Agent 及其原生保存的 Session。无需守护进程、
账号或远程服务，即可检查用量、阅读对话、恢复工作、停止进程并安全删除归档
Session。

- 支持 Claude Code、Codex、Gemini CLI、OpenCode、Pi、Oh My Pi、
  Cursor Agent 和自定义进程识别规则。
- 直接读取 Provider 自己的存储，只报告 Provider 确实持久化的用量与成本。
- 对话保留在本机；日志读取有界、常见 Secret 默认脱敏，破坏性操作执行前会被
  严格验证。

## 安装

```sh
cargo install --git https://github.com/sxwedo/mena --locked
mena --version
```

仓库固定使用 Rust 1.96.1。从源码构建：

```sh
git clone https://github.com/sxwedo/mena.git
cd mena
mise run build
install -m 0755 target/release/mena /usr/local/bin/mena
```

### 给编码 Agent

如果你是 Agent，请执行下面两条命令，将 `mena` 安装为用户级 CLI。不要把它加入
当前项目的 `Cargo.toml`。如果环境中没有 `cargo`，先报告需要安装 Rust，再继续。

```sh
cargo install --git https://github.com/sxwedo/mena --locked
mena sessions --help
```

## 快速开始

```sh
mena ps                         # 列出运行中的 Agent
mena top                        # 监控 CPU、内存和状态
mena sessions                   # 浏览全部已保存 Session
mena ss                         # `mena sessions` 的完全等价简写
mena inspect codex:session-id   # 检查一个进程或 Session
mena resume codex:session-id    # 使用 Provider CLI 恢复
```

Target 稳定且适合脚本使用：

- 运行中进程：`provider:PID`，例如 `claude:43120`。
- 已保存 Session：`provider:session-id`。
- 只有在结果唯一时，才允许省略 Provider。

## 当前重点与路线图

- **当前 - Sessions：** 重点开发 `mena sessions` 及其简写 `mena ss`，包括发现、
  搜索、对话阅读、准确的响应与工具指标、导出、恢复和安全删除。
- **下一步 - Memory：** 后续将关注 `mena memory` 和持久化 Agent 记忆工作流。
  此处只记录方向，相关功能**尚未实现**。

## Sessions

`mena sessions` 与 `mena ss` 完全等价，所有参数都能用于任一形式。

```sh
mena ss
mena ss --provider claude --limit 20
mena ss --plain
mena ss --json
```

交互式 Session 视图可以跨 Provider 和项目搜索，打开完整对话，展示按模型汇总的
持久化指标，恢复原生 Session，将其导出为 Markdown，或永久删除。

| 按键 | 操作 |
|---|---|
| `/` | 按 Target、Provider、项目或标题搜索 |
| `Enter` 或 `i` | 打开 Session 详情 |
| `r` | 使用 Provider 原生 CLI 恢复 |
| `d`，然后输入小写 `y` | 永久删除选中的 Session |
| `c` / `e` | 将完整详情复制 / 导出为 Markdown |
| `Esc` 或 `q` | 关闭或退出 |

在详情视图中，使用方向键或 `j`/`k` 滚动，`PgUp`/`PgDn` 翻页，`Home`/`End`
跳转到首尾，`Shift+↑`/`Shift+↓` 在用户与 Assistant 消息之间跳转。鼠标上报保持
关闭，因此终端原生文本选择仍然可用。

## 命令

| 命令 | 用途 |
|---|---|
| `mena ps [--json]` | 列出已识别的运行中进程 |
| `mena top` | 打开实时资源监控界面 |
| `mena sessions` / `mena ss` | 搜索并管理已保存 Session |
| `mena inspect <target> [--json]` | 检查进程或 Session |
| `mena logs <target> [-n N] [--raw]` | 读取有界的事件日志尾部 |
| `mena resume [target]` | 选择或恢复原生 Session |
| `mena stop <pid-target> [--force]` | 重新验证进程后终止 |
| `mena config init` | 创建私有配置文件 |

使用 `mena <command> --help` 查看全部选项。常用恢复方式：

```sh
mena resume               # 交互式选择
mena resume --last        # 恢复最近更新的 Session
mena resume --list        # 非交互式候选列表
```

## Provider 支持

| Provider | 进程发现 | 已保存 Session | 恢复 | 删除 |
|---|:---:|:---:|:---:|:---:|
| Claude Code | ✓ | ✓ | ✓ | ✓ |
| Codex | ✓ | ✓ | ✓ | ✓ |
| Gemini CLI | ✓ | ✓ | ✓ | ✓ |
| OpenCode | ✓ | ✓ | ✓ | ✓ |
| Pi | ✓ | ✓ | ✓ | ✓ |
| Oh My Pi | ✓ | ✓ | ✓ | ✓ |
| Cursor Agent | ✓ | - | ✓ | - |
| 自定义识别规则 | ✓ | - | 可配置 | - |

Cursor Agent 和自定义识别规则没有通用且受支持的本地 Session 目录。`mena` 会
返回明确的“不支持”错误，而不是猜测存储路径。

### 运行进程与 Session 的关联

只有当前原生证据能够唯一指向一个逻辑 Session 时，`mena` 才展示运行进程的
Session 数据。Claude Code 提供包含 PID、进程启动时间、项目和 Session ID 的原生
运行时记录；Pi 和 Oh My Pi 仅在进程确实打开且只打开一个已收录的原生对话文件时
精确关联（Linux 使用 `/proc`，macOS 使用 `/usr/sbin/lsof`）。

Provider 的恢复或 Session 参数只能证明进程启动时选择了哪个 Session；Agent 可能
在不重启进程的情况下切换 Session，因此该证据标记为 `launch`，而不是 `exact`。
项目相同、时间接近和“最近更新”都不会被用来宣称精确关联。状态为 `ambiguous`、
`unconfirmed` 或 `unsupported` 时，不输出 Session ID、Token 或成本。

## 数据与安全

- 对话数据始终保留在 Provider 的原生本地存储中。
- Token、成本、耗时、TTFT、重试和错误只在原生记录存在时展示，缺失值绝不估算。
- 只有 `exact` 原生关联才会把指标归属到运行进程；`ps --json` 同时输出
  `session_match` 和 `session_match_evidence`。
- 恢复命令使用“程序 + argv”执行，绝不调用 Shell。
- 停止进程前重新验证 PID、启动时间、可执行文件和 Provider。
- 删除操作拒绝运行中的 Session、歧义 Target、路径穿越和 Provider 根目录之外的
  符号链接逃逸；只要某个运行进程无法精确关联，该 Provider 的全部 Session 都按
  fail closed 方式禁止删除。
- 详情导出采用原子写入、绝不覆盖已有文件，并在 Unix 下使用 `0600` 权限。

## 自动化

`ps`、`inspect` 和 `sessions` 提供稳定的 JSON 输出：

```sh
mena ps --json | jq '.[] | {id, project, session_match, session_match_evidence, tokens, cost_usd}'
mena ss --json | jq '.[] | select(.agent == "codex") | .target'
```

人类可读输出中的短横线表示没有精确关联原生 Session；`n/a` 表示已精确关联，但
Session 中没有持久化成本。

## 配置

创建 `~/.config/mena/config.toml`；Unix 下文件权限为 `0600`：

```sh
mena config init
```

基础目录遵循 `XDG_CONFIG_HOME`。自定义进程识别使用精确的可执行文件匹配和可选的
命令标记：

```toml
[agent.custom.my_agent]
executables = ["my-agent"]
command_contains = ["--agent-mode"]
resume = ["my-agent", "resume", "{session}"]
```

恢复命令是 argv 数组，必须包含 `{session}`。已有的 `clix` 自定义 Agent 定义可以
一次性导入：

```sh
mena config init --import-clix
```

正常运行时只读取 `mena` 配置。

## 开发

```sh
mise run verify  # 格式、检查、测试、严格 Clippy 与 rustdoc
mise run build   # 优化后的 release 二进制
```

Provider 适配器和安全敏感变更应包含有针对性的 Fixture。架构与仓库约束参见
[AGENTS.md](AGENTS.md)。

## 许可证

[MIT](LICENSE) © 2026 sxwedo
