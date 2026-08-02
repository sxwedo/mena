<p align="center">
  <img src="https://github.com/user-attachments/assets/b5d7500e-4d16-4e5f-a414-872718010a22" width="220" alt="mena 标志">
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-2563eb.svg" alt="MIT 许可证"></a>
  <a href=".mise.toml"><img src="https://img.shields.io/badge/rust-1.96.1-f97316.svg" alt="Rust 1.96.1"></a>
  <img src="https://img.shields.io/badge/data-local--first-10b981.svg" alt="本地优先">
</p>

<p align="center"><strong>一个快速、本地优先的开发者 Agent 进程与会话控制中心。</strong></p>

<p align="center">
  <a href="README.md">English</a> | 简体中文
</p>

<code>mena</code> 可以发现正在运行的编码 Agent，将它们与各 Provider
原生持久化的 Session 关联起来，展示准确记录的用量，并提供安全的检查、恢复、
停止、导出和删除能力。无需守护进程、账号或远程服务。

- **统一查看多个 Provider。** 支持 Claude Code、Codex、Gemini CLI、
  OpenCode、Pi、Oh My Pi、Cursor Agent，以及自定义进程识别规则。
- **同时覆盖实时与历史上下文。** 查看 CPU、内存、项目、Session、完整对话、
  已持久化 Token、已记录成本、模型及单次响应指标。
- **遵循原生机制。** 直接读取 Provider 自己的本地存储，并通过原生 argv
  恢复 Session，不调用 Shell。
- **默认保护隐私。** 对话数据始终保留在本机；除非显式请求原始输出，否则日志
  读取有界且会进行脱敏。
- **在破坏性操作前严格校验。** 发送进程信号前会重新验证身份；删除 Session
  时遇到正在运行、路径逃逸或选择器歧义都会安全失败。

## 安装

### 从 Git 安装

~~~sh
cargo install --git https://github.com/sxwedo/mena --locked
~~~

### 从源码构建

~~~sh
git clone https://github.com/sxwedo/mena.git
cd mena
mise run build
install -m 0755 target/release/mena /usr/local/bin/mena
~~~

仓库在 <code>.mise.toml</code> 中固定使用 Rust 1.96.1。Provider 的原生
CLI 和 Session 存储都是可选的：mena 只展示当前机器上实际存在的 Provider。

## 60 秒快速开始

~~~sh
# 查看所有已识别的运行中 Agent。
mena ps

# 在响应式终端界面中监控资源使用。
mena top

# 搜索所有已保存的原生 Session，包括非运行状态的 Session。
mena sessions

# 使用稳定选择器检查或恢复 Session。
mena inspect codex:019abcde-session-id
mena resume codex:019abcde-session-id
~~~

Target 稳定且适合脚本使用：

- 运行中进程使用 <code>provider:PID</code>，例如
  <code>claude:43120</code>。
- 持久化 Session 使用 <code>provider:session-id</code>。
- 只有在结果唯一时，才允许省略 Provider。

## 核心工作流

### 监控运行中的 Agent

~~~sh
mena ps
mena ps --json

mena top
mena top --interval 3 --iterations 5
~~~

进程视图展示 <code>ID</code>、<code>AGENT</code>、<code>PROJECT</code>、
<code>STATUS</code>、<code>DURATION</code>、<code>TOKENS</code> 和
<code>COST</code>；实时 TUI 还会展示 CPU 和内存。

| 按键 | 操作 |
|---|---|
| <code>↑</code>/<code>↓</code> 或 <code>j</code>/<code>k</code> | 选择进程 |
| <code>Enter</code> 或 <code>i</code> | 打开详情 |
| <code>r</code> | 刷新 |
| <code>q</code> | 退出 |

### 查找并检查已保存的 Session

~~~sh
mena sessions
mena sessions --provider claude --limit 20
mena sessions --plain
mena sessions --json

mena inspect codex:019abcde-session-id
mena logs claude:session-id -n 100
mena logs claude:session-id -n 20 --raw
~~~

<code>mena sessions</code> 将 <code>TARGET</code> 固定为第一列，并为它
保留最宽的固定宽度，确保完整选择器尽可能可见。终端足够宽时会继续展示 Agent、
项目、标题/摘要和更新时间；窄屏仍会保留 Target 与标题。

| Session 列表按键 | 操作 |
|---|---|
| <code>/</code> | 按 Target、Provider、项目或标题搜索 |
| <code>Enter</code> 或 <code>i</code> | 打开完整详情 |
| <code>r</code> | 通过 Provider CLI 恢复 Session |
| <code>d</code>，然后输入小写 <code>y</code> | 永久删除 |
| <code>Esc</code> 或 <code>q</code> | 关闭或退出 |

详情弹窗会按照 Provider 的原始顺序保留所有已解析消息，不截断长内容。滚动范围
根据实际换行高度计算，不受 16 位行数上限影响。

| 详情按键 | 操作 |
|---|---|
| <code>↑</code>/<code>↓</code> 或 <code>j</code>/<code>k</code> | 滚动 |
| <code>Shift+↑</code>/<code>Shift+↓</code> | 在用户与 Assistant 消息之间跳转 |
| <code>PgUp</code>/<code>PgDn</code> | 按页移动 |
| <code>Home</code>/<code>End</code> | 跳到开头或结尾 |
| <code>c</code> | 将完整详情复制为 Markdown |
| <code>e</code> | 将完整详情导出为 Markdown |
| <code>r</code> | 恢复当前 Session |
| <code>Enter</code> 或 <code>Esc</code> | 返回列表且不丢失选中项 |

鼠标上报保持关闭，因此终端原生的拖动选择和 <code>Command+C</code> 仍然可用。
备用滚动模式会把触控板和滚轮手势映射为导航，同时将点击与拖动继续交给终端处理。
同方向的惯性滚动事件会被合并，防止输入停止后仍因事件积压而持续滚动。

### 恢复与控制

~~~sh
# 交互式选择、恢复最近 Session，或列出候选项。
mena resume
mena resume --last
mena resume --list --limit 20

# 直接恢复指定 Session。
mena resume codex:session-id
mena resume omp:session-id
mena resume cursor:chat-id

# 重新验证进程身份后优雅终止；--force 会强制终止。
mena stop claude:43120
mena stop claude:43120 --force
~~~

恢复命令会被构造成独立的程序与 argv。mena 从不调用 Shell，也不会将任意命令
文本插值后执行。

## Session 详情

### 消息配色

默认配色让对话易于扫读，同时避免过多颜色干扰：

| 类别 | 默认颜色 |
|---|---|
| User 标题与正文 | 浅绿色 |
| Assistant 标题与正文 | 青色 |
| Skill 标题与正文 | 浅黄色 |
| Tool Call、Tool Result、System/Meta 与 Error | 柔和的深灰色 |
| 元数据键 | 浅品红色 |

每种消息的标题和正文颜色都可以独立覆盖，参见[配置](#配置)。

### 模型、耗时与 Token

如果 Provider 持久化了模型 ID，它会显示在对应的 Assistant 消息上。已记录的
响应耗时和请求 Token 会紧随其后：

~~~text
[2026-07-30T13:42:04.795Z] ASSISTANT · gpt-5.6 · 12.3s · 67,890 tokens
Tokens: input 50,000 · output 10,000 · cache read 7,000 · cache write 500 · reasoning 390
~~~

第二行会展示该 Provider 原生记录中实际存在的输入、输出、缓存读取、缓存写入和
推理 Token；缺失字段直接省略，不会猜测。某些 Provider 会把缓存 Token 同时计入
输入 Token，因此原生记录存在明确总数时，mena 会原样保留总数而不会重新计算。
Codex 可以提供已完成 Turn 的耗时和最后一次请求用量；OpenCode 与 Pi 系列记录通常
同时保存这两项指标；Claude Code 和 Gemini 则展示其原生 Session 中实际存在的
字段。Session 总 Token 和成本同样只来自 Provider 自己的持久化记录，mena 绝不会
根据公开模型价格进行估算。

### Markdown 导出

在详情视图中按 <code>e</code>。导出文件包含完整元数据、按顺序排列的消息、
模型 ID、准确的单次响应指标以及结构化 Tool 内容。Assistant Token 明细遵循与
详情弹窗相同的可用字段规则。

- 目标目录：启动 mena 时所在的当前目录。
- 文件名：
  <code>mena-session-{provider}-{safe-id}-{YYYYMMDD-HHMMSS}.md</code>。
- 冲突处理：依次追加 <code>-2</code>、<code>-3</code>；绝不覆盖已有文件。
- 写入保证：原子创建，失败时不会留下不完整文件。
- 隐私权限：Unix 下使用 <code>0600</code>。其他平台仍保证原子写入与不覆盖，
  但不承诺 Unix 权限位。

复制或导出完成后，弹窗会保持打开，并显示结果的绝对路径或可操作的错误信息，
不会改变选中 Session 或滚动位置。

## Provider 支持

| Provider | 进程发现 | 原生 Session 目录 | 恢复 | 永久删除 |
|---|:---:|:---:|:---:|:---:|
| Claude Code | ✓ | ✓ | ✓ | ✓ |
| Codex | ✓ | ✓ | ✓ | ✓ |
| Gemini CLI | ✓ | ✓ | ✓ | ✓ |
| OpenCode | ✓ | ✓ | ✓ | ✓ |
| Pi | ✓ | ✓ | ✓ | ✓ |
| Oh My Pi | ✓ | ✓ | ✓ | ✓ |
| Cursor Agent | ✓ | — | ✓ | — |
| 自定义 Agent | ✓ | — | 可配置 | — |

Cursor Agent 和自定义识别规则没有通用且受支持的本地 Session 目录。对于归档
Session 操作，mena 会明确返回“不支持”错误，而不是猜测存储路径。

## 命令参考

| 命令 | 用途 |
|---|---|
| <code>mena ps [--json]</code> | 列出已识别的运行中进程 |
| <code>mena top</code> | 监控 CPU、内存、状态与准确的持久化用量 |
| <code>mena inspect &lt;target&gt; [--json]</code> | 检查运行中进程或已保存 Session |
| <code>mena logs &lt;target&gt; [-n N] [--raw]</code> | 读取有界的事件日志尾部 |
| <code>mena sessions</code> | 搜索、检查、恢复、导出或删除 Session |
| <code>mena stop &lt;pid-target&gt; [--force]</code> | 重新验证进程身份后终止进程 |
| <code>mena resume [session-target]</code> | 通过 Provider 原生 CLI 恢复 Session |
| <code>mena config init</code> | 创建私有配置文件 |

使用 <code>mena &lt;command&gt; --help</code> 查看每个命令的完整选项。

### 自动化输出

<code>mena ps --json</code>、<code>mena inspect --json</code> 和
<code>mena sessions --json</code> 会输出稳定的机器可读字段。

~~~sh
mena ps --json | jq '.[] | {id, project, tokens, cost_usd}'
mena sessions --json | jq '.[] | select(.agent == "codex") | .target'
~~~

在人类可读的进程输出中，短横线表示未关联到原生 Session；<code>n/a</code>
表示 Session 存在，但其中没有准确记录的成本。

## 配置

创建权限受限的 <code>~/.config/mena/config.toml</code>：

~~~sh
mena config init
~~~

Unix 下文件权限为 <code>0600</code>。基础目录遵循
<code>XDG_CONFIG_HOME</code>。

### 自定义进程识别规则

~~~toml
[agent.custom.my_agent]
executables = ["my-agent", "my-agent.exe"]
command_contains = ["--agent-mode"]
resume = ["my-agent", "resume", "{session}"]
~~~

可执行文件基名采用精确匹配。每条恢复命令都是 argv 数组，并且必须包含
<code>{session}</code> 占位符。

### 详情颜色

所有键均为可选；未配置的键继续使用默认值。

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

颜色值支持 <code>reset</code>、标准 ANSI 颜色名称、形如
<code>ansi:45</code> 的索引色，或形如 <code>#a1b2c3</code> 的真彩色。
无效值会在启动时携带准确配置路径报错，不会静默回退。编辑文件后请重新启动 mena。

## 性能与安全模型

- Session 目录扫描只保留识别所需的元数据；用量与完整对话保持惰性加载。
- 准确用量按需读取，并缓存到对应存储文件发生变化为止。
- 打开详情只进行一次 Provider 原生遍历，同时生成汇总用量和完整有序对话；
  不会为了总量再次读取 Session。
- JSON 文档、单条 JSONL 记录、日志尾部与目录扫描都有明确上限。
- 默认输出会脱敏常见进程参数中的 Secret；<code>--raw</code> 是读取原始记录
  的显式选择。
- 停止进程前会立即重新验证 PID、启动时间、可执行文件和 Provider。
- 删除操作会验证存储 ID 与规范化根目录，拒绝符号链接或路径穿越逃逸，删除已知的
  Provider Sidecar/索引，并拒绝删除仍与运行中进程关联的 Session。
- 破坏性确认只接受小写 <code>y</code>。

## 故障排查

**Cursor Agent 或自定义识别规则没有显示已保存 Session。** 它们的进程可以被
发现和恢复，但 mena 不会猜测原生归档路径。

**Token 或成本缺失。** mena 只报告原生记录中实际存在的准确值，不推断用量或价格。

**在终端中拖动选择后无法复制。** 使用终端原生选择手势和平台复制快捷键。如果需要
一次复制完整详情，请按 <code>c</code> 将其序列化为 Markdown。

**无法完整打开某个对话。** 该原生 JSONL 记录超过安全上限。mena 会携带受影响的
文件路径报错，而不会静默展示不完整对话。

**恢复失败。** 确认所选 Provider CLI 已安装并可从 <code>PATH</code> 找到，
然后使用包含 Provider 的完整 Target 重试。

## 架构

~~~text
src/
├── main.rs           CLI 入口与退出处理
├── lib.rs            命令定义与分发
├── controller.rs     编排、Target 解析、JSON 与原生恢复
├── process.rs        进程发现、识别、资源采样与安全终止
├── session.rs        目录发现、有界 I/O 与删除安全
├── session/
│   └── detail.rs     单次遍历的 Provider 用量与对话适配器
├── export.rs         完整 Markdown 渲染与防冲突导出
├── tui.rs            响应式进程和 Session 界面
├── view.rs           稳定的纯文本表格与格式化
├── settings.rs       私有 mena 配置与 UI 偏好
├── fs.rs             原子替换与私有、不覆盖创建
└── ui.rs             终端状态与错误展示
~~~

对外模块保持精简。Provider 特有的持久化 Schema 隐藏在 Session 详情适配器
之后；文件系统变更统一通过经过验证的删除接口和原子写入接口执行。

## 开发与贡献

~~~sh
mise run verify  # 格式、类型检查、测试、严格 Clippy 与 rustdoc
mise run build   # 优化后的 release 二进制
~~~

变更应保持稳定的选择器与 JSON 字段，为 Provider 原生存储行为补充 Fixture，
并让破坏性操作保持安全失败。提交 Pull Request 前：

1. 添加或更新有针对性的测试。
2. 运行 <code>mise run verify</code>。
3. 对影响 release 的变更运行 <code>mise run build</code>。
4. 用户可见行为变化时同步更新中英文 README。

欢迎提交 Issue 和范围明确的 Pull Request。报告安全敏感问题时，请勿在公开内容中
附带原生 Session 内容、进程参数或文件系统路径。

## 许可证

[MIT](LICENSE) © 2026 sxwedo
