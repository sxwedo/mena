# 配置

`mena` 读取 `~/.config/mena/config.toml`；设置 `XDG_CONFIG_HOME` 时会遵循该基础
目录。

## 创建配置

```sh
mena config init
```

Unix 下文件权限为 `0600`，已有文件绝不覆盖。

用 `t` 或 `mena ss rename` 设置的 Session 显示标题另外写在同一目录下的
`session-titles.toml`。这是 mena 自己的元数据，不会改写 Provider 的 Session
文件。overlay 清空后文件会被删除。Unix 下新建 overlay 的权限为 `0600`。

## 自定义 Agent

```toml
[agent.custom.my_agent]
executables = ["my-agent"]
command_contains = ["--agent-mode"]
resume = ["my-agent", "resume", "{session}"]
```

- `executables` 是需要精确匹配的可执行文件名。
- 只有全部 `command_contains` 标记都出现在 argv 中，进程才算匹配。
- `resume` 是 argv 数组，不是 Shell 文本，并且必须包含 `{session}`。
- 自定义 Agent 可以启动和识别，但没有可猜测的通用 Session 存储适配器。

无效定义会在进程发现或启动前失败。

## Session 详情颜色

所有颜色均为可选项。支持 `cyan`、`light-magenta` 等 ANSI 名称、`ansi:0` 到
`ansi:255` 的索引色，以及 `#7dd3fc` 形式的 RGB 值。

```toml
[ui.session_detail.colors]
border = "#7ca7d9"
metadata_key = "#7ca7d9"
user_content = "#e1e6eb"
assistant_content = "#e1e6eb"
tool_call_content = "#a8b0ba"
error_content = "#d97b84"
```

运行 `mena config init` 生成的模板包含全部可用颜色键及默认值。这些默认值与统一的
[Calm Console 界面](interface_CN.md)一致；已有配置中的显式颜色仍优先生效。
