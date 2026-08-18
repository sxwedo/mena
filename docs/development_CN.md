# 架构与开发

`mena` 是独立的 Rust 2024 crate，固定使用 Rust 1.96.1。

## 数据流

```text
main.rs
  └── lib.rs                 命令定义与分发
      ├── controller.rs      命令编排
      ├── continuation.rs    跨 Agent 导入与 handoff 策略
      ├── process.rs         Provider 识别与运行证据
      ├── session.rs         Provider 无关模型与安全控制
      │   └── session/adapter.rs
      │       ├── storage.rs 原生布局与索引清理
      │       └── detail.rs  对话记录归一化
      ├── skill.rs           Skill 目录、唯一选择与读取范围
      │   └── skill/adapter
      │       ├── storage.rs 发现与目录读取
      │       └── detail.rs  有界文本读取与 frontmatter 解析
      ├── mcp.rs             注册模型、目录与 probe gate
      │   ├── adapter.rs     闭合的客户端配置发现 seam
      │   │   ├── common.rs  归一化与脱敏
      │   │   ├── edit.rs    来源定位与原生配置变更
      │   │   ├── storage.rs 有界配置 I/O
      │   │   ├── codex.rs / json_clients.rs / goose.rs
      │   │   └── plugins.rs 已启用 Plugin 发现与路径范围
      │   └── probe.rs       显式协议元数据发现
      ├── tui
      │   ├── agent_launcher
      │   ├── mcp
      │   ├── session
      │   └── skill
      ├── settings.rs
      ├── editor.rs          不经 Shell 的外部编辑器启动
      ├── export.rs / clipboard.rs / fs.rs
      └── view.rs / ui.rs
```

Provider Session 通过闭合枚举形成 adapter seam。内置 Provider 都在编译期确定，
因此新增枚举分支时，发现、关联、详情、恢复与删除会保持穷举检查。

跨 Agent 继续使用 `continuation.rs` 中独立的 seam，集中管理目标矩阵、原生导入 argv、
临时私有 handoff 和新 Session Prompt。Session TUI 只返回 `Resume` 或
`ContinueWith`，不解析 Transcript，也不构造 Provider 命令。

Skill Catalog 是 Skill 发现和预览的文件系统 seam。TUI 只消费 Catalog 结果和已
缓存的目录条目，不直接读取任意路径。

MCP Catalog 同样把公共注册元数据与私有连接材料分开。Adapter 在 scan 时只读取与
归一化；只有调用方显式要求 live probe 后，raw command、env、header 与 URL 值才能
进入 `probe.rs`。

MCP TUI 负责分组搜索、选择、Spotlight 详情渲染缓存、来源动作、删除确认和有界 Probe
工作线程。工作线程仍通过 `McpCatalog` 回调，不接收也不重建 Adapter 的私有连接值。

MCP 配置修改也通过同一个 Catalog seam。来源行定位和删除都会重新读取当前原生文件；
删除按客户端结构完成校验，再通过 `fs.rs` 原子写入并保留权限。外部编辑器不经过
Shell 启动，退出后 TUI 通过 Catalog 刷新。TUI 不直接解析或写入 Provider 配置。

## 安全不变量

- 原生恢复命令必须分别构造程序和 argv，绝不调用 Shell。
- 跨 Agent 路径必须显式：有 Provider 原生能力时使用导入参数，其余路径通过临时私有
  handoff 新建 Session。
- 恢复 argv 只属于启动证据，不能代表当前 Session 身份。
- 不能根据项目相同或更新时间推断精确运行关联。
- active 展示只接受精确证据；证据缺失或歧义时，删除保护必须更保守。
- 对话与 Skill 读取必须有界。
- 删除前验证 ID、规范路径和 Provider 根目录包含关系。
- 不估算 Token 成本，也不虚构单次 Tool Token。
- 自定义 Agent 不允许猜测 Session 目录。
- 静态 MCP scan 绝不启动进程或访问 Server。
- MCP probe 不调用 Tool、不读取 Resource、不展开 Prompt。
- MCP 发现不得调用 Shell 或动态凭据 helper。
- MCP 敏感值必须保持私有；只序列化脱敏目标以及 binding 名称/来源。
- 绝不能把脱敏占位符写入文件；managed/plugin 来源不能编辑或删除，带注释格式不能
  自动删除。
- MCP 配置写入必须重新读取来源、保留无关数据、验证原生结构并原子替换文件。
- Server 描述、schema 与 safety annotation 都按不可信数据处理。

## 增加 Provider 支持

1. 在 `process.rs` 中增加识别与原生可执行文件行为。
2. 扩展闭合的 `ProviderAdapter` 枚举。
3. 将原生路径和索引清理放入 `session/adapter/storage.rs`。
4. 在 `session/adapter/detail.rs` 中归一化原生记录。
5. 为发现、用量、详情、关联、歧义、删除和根目录包含增加接口级 Fixture。
6. 更新 README Provider 矩阵和相关文档。

不要仅为了增加编译期内置 Provider 就引入运行时注册或 `dyn Trait`。

## 增加 MCP 客户端支持

1. 扩展 `mcp/adapter.rs` 中的闭合发现序列。
2. 原生解析放在单一职责 Adapter，并通过 `mcp/adapter/common.rs` 归一化。
3. 保持可序列化 `McpRegistration` 与私有 `McpConnection` 分离，凭据值绝不能进入
   公共模型。
4. 为每种原生来源/scope、歧义、未知字段、脱敏、环境展开与错误 transport 增加
   接口测试。
5. Live transport 只能放进 `mcp/probe.rs`，并保持显式 opt-in、时间/数量/分页
   上限、错误脱敏与零 Tool 调用。
6. 若客户端来源可写，在 `mcp/adapter/edit.rs` 增加原生写回行为与接口测试；不得猜测
   通用 enable 字段。
7. 更新 [MCP 来源与元数据矩阵](mcp_CN.md)。

## 验证

所有门禁都定义在 `.mise.toml`：

```sh
mise run fmt
mise run check
mise run test
mise run clippy
mise run docs
mise run verify
mise run build
```

`mise run verify` 包括格式检查、类型检查、全部测试、warnings denied 的 pedantic / 
nursery Clippy，以及 warnings denied 的 rustdoc。
