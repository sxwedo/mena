# MCP 目录

`mena mcp` 把两个证据层明确分开：

1. **配置了什么？** 默认命令只读取本地客户端配置和已启用插件清单，不连接
   Server，也不启动进程。
2. **Server 此刻暴露了什么？** 在浏览器按 `p`，或使用 `inspect --probe`，会显式
   初始化一个 Server 并读取其声明的目录；它不调用工具、不读取资源内容，也不展开
   Prompt。

## 命令

```sh
mena mcp
mena mcp --json
mena mcp --provider codex --scope project
mena mcp --source .codex/config.toml

mena mcp --provider codex inspect codegraph
mena mcp --provider codex inspect codegraph --json
mena mcp --provider codex inspect codegraph --probe --timeout 15
```

在交互终端中，`mena mcp` 会打开可搜索的双栏浏览器；初始详情完全来自静态配置。
管道输出仍是普通表格，`--json` 始终输出机器可读数据。

浏览器快捷键：

- `/`：按名称、selector、客户端、scope、transport、目标、描述、配置工具和来源搜索；
- `↑`/`↓` 或 `j`/`k`：在列表移动；切换到详情栏后独立滚动；
- `Tab`、`←`/`→` 或 `h`/`l`：切换栏位，不改变选中项；
- `Enter`：切换全屏详情；
- `p`：在工作线程显式 probe 当前注册，列表仍可继续操作；即使选择移动，结果也只会
  绑定到发起 probe 的注册；
- `q`/`Esc`：返回或退出。Probe 运行中退出时，Mena 会等待有界清理完成，避免遗留
  stdio 子进程。

筛选条件会在 inspect 前生效：

- `--provider`：`claude`、`codex`、`cursor`、`gemini`、`goose`、`omp`、
  `opencode` 或 `pi`；
- `--scope`：`user`、`local`、`project`、`plugin`、`profile`、`managed` 或
  `shared`；
- `--source`：完整配置路径或路径后缀；
- `--json`：稳定的机器可读输出。

常规 selector 是 `provider:scope:name`。Mena 会保留所有来源，不模拟各客户端的
覆盖优先级。如果两个来源仍得到同一个 selector，inspect 会按歧义失败并列出路径；
使用 `--source` 精确选择。

这里审计的是“配置声明”，不是对客户端最终合并运行状态的复刻。客户端 trust
规则、managed policy、命令行覆盖或运行中缓存仍可能屏蔽一条记录；`enabled` 表示
该来源中声明的状态。

## 配置来源

项目配置会从当前目录向 home 方向查找，并采用最近的匹配文件。

| 客户端 | Scope | 记录来源 |
|---|---|---|
| Claude Code | user | `~/.claude.json` → `mcpServers` |
| Claude Code | local | `~/.claude.json` → 最近匹配的 `projects.<path>.mcpServers` |
| Claude Code | project | 最近的 `.mcp.json` |
| Claude Code | plugin | 仅已安装且已启用插件中的 MCP 清单 |
| Claude Code | managed | 当前平台的系统级 `managed-mcp.json` |
| Codex | user | `~/.codex/config.toml` → `[mcp_servers.*]` |
| Codex | project | 最近的 `.codex/config.toml` → `[mcp_servers.*]` |
| Codex | plugin | 本地 marketplace 中已启用插件的 `.mcp.json` |
| Cursor | user / project | `~/.cursor/mcp.json`、最近的 `.cursor/mcp.json` |
| Gemini CLI | user / project | `~/.gemini/settings.json`、最近的 `.gemini/settings.json` |
| OpenCode | user / project | `~/.config/opencode/opencode.json[.c]`、最近的 `opencode.json[.c]`；兼容 v1/v2 |
| Goose | user | `~/.config/goose/config.yaml` → `extensions` |
| Oh My Pi | user / project | `~/.omp/agent/mcp.json`、最近的 `.omp/mcp.json` |
| Oh My Pi | profile | `~/.omp/profiles/*/agent/mcp.json` |
| Pi adapter | user / shared / project | `~/.pi/agent/mcp.json`、`~/.config/mcp/mcp.json`、最近的 `.pi/mcp.json` / `.mcp.json` |

Pi 本身没有原生 MCP 目录。只有当 `~/.pi/agent/settings.json` 中出现
`pi-mcp-adapter` package 时，Mena 才扫描 Pi 路径，避免把无关 `.mcp.json` 误标为
Pi 配置。

Goose 把外部 MCP Server 与 Provider 原生 Extension 放在同一个 registry。Mena
忠实记录 stdio、Streamable HTTP、SSE、builtin、platform、frontend 与
inline-Python；只有外部 stdio 和 Streamable HTTP 可以实时 probe。

Claude managed 配置路径：

- macOS：`/Library/Application Support/ClaudeCode/managed-mcp.json`
- Linux/WSL：`/etc/claude-code/managed-mcp.json`
- Windows：`%ProgramFiles%\ClaudeCode\managed-mcp.json`

只存在于云端 Connector、App-backed plugin 或运行中客户端内存里的 Server，不会被
猜成“本地注册”；只有存在可读的本地 transport 定义时才进入目录。

## 静态注册元数据

每条注册记录：

- 身份：selector、name、客户端 Provider、scope、来源路径与语法；
- 状态：enabled、结构是否合法、warning 与未知字段名；
- transport：stdio、Streamable HTTP、SSE 或 Provider 原生类型；
- 启动目标：脱敏后的 command、argv、URL、工作目录与 timeout；
- 认证元数据：认证方式与凭据引用；
- 值绑定：环境变量/header 的**名称**、来源类型与敏感提示，JSON 中不输出字面值；
- 工具策略：include/exclude 与 approval mode；
- 安全的 Provider option，例如 trust、codemode、plugin ID/version、OMP profile；
  Codex placement、OAuth resource/scopes，以及毫秒或小数秒 timeout 也会归一化；
- 可选 display name 与 description。

`extra_fields` 只记录尚未归一化的配置**键名**。值会被省略，因为新字段可能承载
凭据。

已知敏感位置会脱敏：省略 env/header 值，移除 URL userinfo、query value 和
fragment，bearer token 只保留环境变量名，常见 secret/header argv flag 会隐藏后续
参数。name 与 description 属于用户自定义文本；不要把凭据放进去。

## 实时协议元数据

`--probe` 增加 `probe` 对象，包含：

- 状态与耗时；
- 协商后的 MCP protocol version；
- Server name/title/version/description/website/instructions；
- tools/prompts/resources capability、list-change、subscription、logging、completion、
  experimental 与 extension ID；
- `tools/list`：名称、标题、描述、输入/输出 schema、安全提示、配置过滤/approval
  结果及协议 metadata 键名；
- `prompts/list`：名称、标题、描述与参数定义；
- `resources/list`：URI、名称、标题、描述、MIME type 与大小；
- `resources/templates/list`：URI template 与描述字段；
- 部分目录失败 warning，或脱敏后的连接/协议错误。

Probe 状态：

| 状态 | 含义 |
|---|---|
| `success` | 初始化和所有已声明 list 操作均成功 |
| `partial` | 初始化成功，但目录或清理步骤部分失败 |
| `failed` | 进程、网络、认证、超时或协议失败 |
| `refused` | 注册被禁用或结构非法 |
| `unsupported` | 有静态 transport，但 Mena 没有安全的实时实现 |

`inspect --probe` 遇到 `failed`、`refused`、`unsupported` 时，会先输出详情，再以
非零状态退出；交互浏览器会原位展示状态，并允许继续检查或重试。

### Probe transport 与认证支持

- **stdio**：程序与 argv 分开传递，不调用 Shell；子进程只获得精简基础环境以及
  显式配置/转发的值。
- **Streamable HTTP**：支持静态 header、环境变量 header、bearer-token env 引用。
- **SSE 与 Provider 原生类型**：只做静态记录，不 probe。
- **OAuth/Provider credential store**：记录认证类型，但不抽取其他客户端 token，也
  不启动 OAuth flow；受保护 endpoint 可能返回认证失败。
- **动态 header/value helper**：只记录、绝不执行；必须改为环境变量引用后才能
  probe。
- **Codex remote executor placement**：单独标记 remote environment binding；由于
  Mena 无法安全复刻 Codex remote executor，这类注册保持仅静态可见。

按 `p` 或使用 `--probe` 会执行本地配置的代码或访问配置 endpoint。对 project
scope 尤其应先看静态 inspect。

## 有界性与安全契约

- 单配置读取上限 8 MiB、单文件最多 10,000 条注册；
- 已安装 Plugin 根目录和被引用的 MCP 清单路径先 canonicalize，且必须留在 Plugin
  cache/marketplace 根目录内；
- Probe timeout 必须在 1–300 秒；
- 每类运行时目录最多 10,000 项、1,000 页；重复 cursor 直接失败；
- 文本元数据最多 64 KiB，单个 JSON schema 最多 1 MiB；
- stdio 清理有界，transport drop 时会终止 child；
- Probe error 有长度上限，并会擦除原始 URL、参数、header 与已解析凭据；
- Server 元数据与 Tool safety annotation 都是不可信声明，只记录、不用于授权。

## 实现位置

MCP 功能是一个小接口、深实现的模块：

| 文件 | 职责 |
|---|---|
| `src/lib.rs` | CLI 参数与命令分发 |
| `src/controller.rs` | scan/filter/inspect 编排和退出语义 |
| `src/tui/mcp/` | 搜索、栏位导航、详情缓存与 Probe 工作线程 |
| `src/mcp.rs` | 公共模型、排序、筛选、消歧与 probe gate |
| `src/mcp/adapter.rs` | 闭合发现 seam 与私有连接材料 |
| `src/mcp/adapter/storage.rs` | 有界读取、最近项目查找、profile 上限 |
| `src/mcp/adapter/codex.rs` | Codex 原生 TOML 归一化 |
| `src/mcp/adapter/json_clients.rs` | Claude、Cursor、Gemini、OpenCode、OMP、Pi 格式 |
| `src/mcp/adapter/goose.rs` | Goose YAML Extension 归一化 |
| `src/mcp/adapter/plugins.rs` | 已启用 Claude/Codex Plugin 的发现与路径约束 |
| `src/mcp/adapter/common.rs` | 公共归一化、脱敏、raw/public 分离 |
| `src/mcp/probe.rs` | 显式 rmcp client、transport、目录上限、运行时模型 |
| `src/view.rs` | 人类可读表格与 detail |

测试与公开 seam 同处，覆盖注册归一化、凭据脱敏（包括安全的 `Debug` 输出）、同名
消歧、Plugin 启用/路径范围与 wrapped/top-level 清单、动态 helper 拒绝、remote
executor 拒绝、disabled Server 拒绝，以及一个内存 MCP Server；后者会断言元数据
发现过程中 Tool 调用次数严格为 0。

## 上游格式参考

- [Codex MCP 配置](https://developers.openai.com/codex/mcp/)
- [Claude Code MCP 配置](https://code.claude.com/docs/en/mcp)
- [Gemini CLI MCP 配置](https://google-gemini.github.io/gemini-cli/docs/tools/mcp-server.html)
- [OpenCode MCP 配置](https://opencode.ai/v2/docs/mcp-servers)
- [Oh My Pi MCP 配置](https://github.com/can1357/oh-my-pi/blob/main/docs/mcp-config.md)
- [Goose 配置](https://github.com/aaif-goose/goose/blob/main/documentation/docs/guides/config-files.md)
- [Rust MCP SDK](https://github.com/modelcontextprotocol/rust-sdk/blob/main/crates/rmcp/README.md)
