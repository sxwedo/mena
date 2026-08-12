# Configuration

`mena` loads `~/.config/mena/config.toml`. The base directory follows
`XDG_CONFIG_HOME` when it is set.

## Create the file

```sh
mena config init
```

On Unix, the file is created with mode `0600` and is never overwritten. To copy
only legacy `[agent.custom]` entries from clix once:

```sh
mena config init --import-clix
```

Normal command execution reads mena configuration only; the clix file is not a
runtime dependency.

## Custom agents

```toml
[agent.custom.my_agent]
executables = ["my-agent"]
command_contains = ["--agent-mode"]
resume = ["my-agent", "resume", "{session}"]
```

- `executables` contains exact executable names.
- Every `command_contains` marker must appear in argv for a process match.
- `resume` is an argv array, not shell text, and must contain `{session}`.
- Custom agents can be launched and recognized but have no generic session
  storage adapter.

Invalid custom definitions fail before process discovery or launch.

## Session-detail colors

Every color is optional. Accepted values are ANSI names such as `cyan` or
`light-magenta`, indexed colors `ansi:0` through `ansi:255`, and RGB values such
as `#7dd3fc`.

```toml
[ui.session_detail.colors]
border = "cyan"
metadata_key = "light-magenta"
user_content = "light-green"
assistant_content = "cyan"
tool_call_content = "dark-gray"
error_content = "red"
```

Run `mena config init` to generate a template containing every supported color
key and its default.
