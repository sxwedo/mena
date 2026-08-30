<p align="center">
  <img src="https://github.com/user-attachments/assets/1fb078f4-f1e1-4196-b97e-162505a6eafe" width="220" alt="mena logo"/>
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

`mena` 是一个本地优先的 CLI，用来启动编码 Agent、浏览原生 Session、检查
Agent Skill，以及审计 MCP 注册。无需守护进程、账号或远程数据存储。

## 安装

需要 Rust 1.96.1 或更高版本；`cargo install` 会从源码编译。

```sh
cargo install --git https://github.com/sxwedo/mena --locked
mena --version
```

从仓库源码构建：

```sh
git clone https://github.com/sxwedo/mena.git
cd mena
mise run build
```

## 快速开始

```sh
mena agent                    # 在当前目录选择并启动 Agent
mena ag claude                # 启动 Claude Code
mena ag codex --resume        # 恢复当前项目最近的 Codex Session
mena ps                       # 列出运行中的编码 Agent 进程
mena sessions                 # 浏览已保存 Session
mena ss --provider cursor     # 按 Provider 筛选 Session
mena skills                   # 浏览已安装的 Agent Skill
mena sk inspect ponytail      # 检查唯一命名的 Skill
mena mcp                      # 交互浏览 MCP 注册
mena mcp open codegraph       # 打开来源配置并定位到该注册
mena mcp inspect codegraph --probe  # 显式发现实时 MCP 元数据
mena memories                 # 列出 Agent 记忆文件
mena ms inspect CLAUDE.md     # 读取唯一命名的记忆文件
```

| 命令 | 用途 |
|---|---|
| `mena agent` / `mena ag` | 启动 Agent，支持新建或恢复原生 Session |
| `mena ps` | 列出运行中的编码 Agent 进程 |
| `mena sessions` / `mena ss` | 搜索、检查、恢复、换 Agent 继续、导出和删除 Session |
| `mena skills` / `mena sk` | 列出、筛选、检查和浏览 Agent Skill |
| `mena mcp` | 分组浏览注册，打开/编辑/删除配置，并显式探测元数据 |
| `mena memories` / `mena ms` | 列出、读取、编辑和删除 Agent 记忆文件 |
| `mena config init` | 创建 `~/.config/mena/config.toml` |

## Provider 支持

| Provider | 启动 | Session | 换 Agent 继续 | MCP 配置 |
|---|:---:|:---:|---|:---:|
| Claude Code | ✓ | ✓ | 导入 OMP；handoff 到 Codex | ✓ |
| Codex | ✓ | ✓ | 导入 OMP；handoff 到 Claude | ✓ |
| Gemini CLI | ✓ | ✓ | handoff 到 Claude/Codex/OMP | ✓ |
| OpenCode | ✓ | ✓ | handoff 到 Claude/Codex/OMP | ✓ |
| Pi | ✓ | ✓ | handoff 到 Claude/Codex/OMP | adapter¹ |
| Oh My Pi | ✓ | ✓ | handoff 到 Claude/Codex | ✓ |
| Cursor Agent | ✓ | ✓ | handoff 到 Claude/Codex/OMP | ✓ |
| Goose | ✓ | — | — | ✓ |
| 自定义配置 | ✓ | — | — | — |

¹ 仅在安装 `pi-mcp-adapter` 后发现 Pi 条目。

自定义 Agent 和 Goose 没有通用 Session 目录。`mena` 会明确返回不支持，而不会
猜测 Provider 自己的存储路径。
在 Session 浏览器中，小写 `r` 执行原生恢复，大写 `R` 选择一个已安装的目标
Agent。导入与 handoff 的具体语义见 Session 文档。

## 文档

- [Calm Console 终端界面与响应式行为](docs/interface_CN.md)
- [Session 浏览、指标、导出与删除](docs/sessions_CN.md)
- [Agent Skill 发现与浏览](docs/skills_CN.md)
- [MCP 目录、元数据、来源与安全边界](docs/mcp_CN.md)
- [Agent 记忆文件](docs/memories_CN.md)
- [配置](docs/configuration_CN.md)
- [架构与开发](docs/development_CN.md)

## 安全模型

- Session 数据始终保留在各 Provider 的原生本地存储中。
- 只有 Provider 持久化的用量和成本才会展示；不会根据公开价格推算。
- 恢复命令使用“程序 + argv”，绝不调用 Shell。
- 跨 Agent handoff 使用临时私有 Markdown 并新建目标 Session；不会把 Provider
  运行状态伪装成已迁移。
- `mena ps` 是一次性只读的 OS 进程快照；其中的状态不是推断出的 Agent 生命周期，
  `--verbose` 会打印可能包含密钥的完整命令行。
- MCP 浏览器初始只展示静态配置；只有按 `p` 或使用 `--probe` 才启动 stdio 或访问
  HTTP，且 probe 不调用工具、不读取资源。
- 运行进程与 Session 的关联必须来自 Provider 原生证据；不确定时删除操作按
  fail-closed 处理。
- 删除前会验证 Session ID、规范路径、符号链接范围和运行进程保护状态。
- 记忆发现是静态且有读取上限的；编辑通过配置的编辑器完成，删除需要显式确认
  并校验根目录包含关系。

完整约束见 [Session 安全说明](docs/sessions_CN.md#安全与运行中-session-关联)。

## 开发

```sh
mise run verify  # 格式、检查、测试、严格 Clippy 与 rustdoc
mise run build   # 优化后的 release 二进制
```

仓库架构与扩展入口见 [docs/development_CN.md](docs/development_CN.md)，面向编码 Agent 的
不变量见 [AGENTS.md](AGENTS.md)。

## 许可证

[MIT](LICENSE) © 2026 sxwedo
