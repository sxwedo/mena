# 架构与开发

`mena` 是独立的 Rust 2024 crate，固定使用 Rust 1.96.1。它虽然从 clix workspace
迁出，但不能重新引入指向 clix 的路径依赖。

## 数据流

```text
main.rs
  └── lib.rs                 命令定义与分发
      ├── controller.rs      命令编排
      ├── process.rs         Provider 识别与运行证据
      ├── session.rs         Provider 无关模型与安全控制
      │   └── session/adapter.rs
      │       ├── storage.rs 原生布局与索引清理
      │       └── detail.rs  对话记录归一化
      ├── skill.rs           Skill 目录、唯一选择与读取范围
      │   └── skill/adapter
      │       ├── storage.rs 发现与目录读取
      │       └── detail.rs  有界文本读取与 frontmatter 解析
      ├── tui
      │   ├── agent_launcher
      │   ├── session
      │   └── skill
      ├── settings.rs
      ├── export.rs / clipboard.rs / fs.rs
      └── view.rs / ui.rs
```

Provider Session 通过闭合枚举形成 adapter seam。内置 Provider 都在编译期确定，
因此新增枚举分支时，发现、关联、详情、恢复与删除会保持穷举检查。

Skill Catalog 是 Skill 发现和预览的文件系统 seam。TUI 只消费 Catalog 结果和已
缓存的目录条目，不直接读取任意路径。

## 安全不变量

- 原生恢复命令必须分别构造程序和 argv，绝不调用 Shell。
- 恢复 argv 只属于启动证据，不能代表当前 Session 身份。
- 不能根据项目相同或更新时间推断精确运行关联。
- active 展示只接受精确证据；证据缺失或歧义时，删除保护必须更保守。
- 对话与 Skill 读取必须有界。
- 删除前验证 ID、规范路径和 Provider 根目录包含关系。
- 不估算 Token 成本，也不虚构单次 Tool Token。
- 自定义 Agent 不允许猜测 Session 目录。

## 增加 Provider 支持

1. 在 `process.rs` 中增加识别与原生可执行文件行为。
2. 扩展闭合的 `ProviderAdapter` 枚举。
3. 将原生路径和索引清理放入 `session/adapter/storage.rs`。
4. 在 `session/adapter/detail.rs` 中归一化原生记录。
5. 为发现、用量、详情、关联、歧义、删除和根目录包含增加接口级 Fixture。
6. 更新 README Provider 矩阵和相关文档。

不要仅为了增加编译期内置 Provider 就引入运行时注册或 `dyn Trait`。

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
