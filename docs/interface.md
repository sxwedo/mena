# Terminal interface

`mena` uses a shared **Calm Console** interface across the agent launcher,
Session browser, Skill browser, and MCP registry. It is designed for sustained,
keyboard-first work: clear enough to scan quickly, restrained enough to leave
open all day.

## Visual language

- `mena · Section` names the current area in plain language. Panels use short,
  descriptive titles such as `Sessions`, `Skills`, and `Details`.
- Large surfaces are neutral charcoal. A low-saturation steel blue marks focus;
  muted green means success, soft amber means caution, and muted red is reserved
  for errors and destructive confirmation.
- Borders, table headers, and metadata stay quieter than primary content. Small
  dot labels communicate state without filling large areas with color.
- Decorative motion is omitted. Only real in-progress work may animate, so the
  interface remains stable while it is being read.
- Key hints use the same `[key] action` grammar on every screen. Narrow
  terminals show the primary actions instead of clipping a long footer.
- Text-mode status output uses the natural prefixes `mena:`, `done:`, and
  `error:`.

Provider symbols retain subtle identity colors inside the shared shell. These
are identity cues only; operational state uses the semantic colors above.

## Shared keymap

The same key means the same thing in every browser:

- `/` always filters the loaded catalog fields. It never scans transcripts and
  never starts a process.
- `Enter` opens the primary view for the selection.
- `o` opens the native artifact: the Skill directory, the MCP source config,
  or the memory file in the configured editor.
- `e` edits a native configuration; it never exports.
- `p` is the only key that can start a process or contact an endpoint, and it
  always shows a confirmation naming the selector, the source, and what will
  run before anything starts.
- `d` deletes. The confirmation states the exact consequence — trash with a
  restore path for sessions, an irreversible rewrite for MCP and memory files.
- `?` opens a full keymap overlay for the current surface; any key closes it.
- `1`–`4` jump between Sessions, Skills, MCP, and Memories without returning
  to the shell.

## Responsive layout

The interface adapts from the frame size on every redraw:

- below 100 columns, the Skill list/preview, MCP list/details, and memory
  list/preview panes stack vertically;
- below 88 columns, the launcher hides its lower-priority session-context
  column;
- below 96 columns, footers collapse to the primary key hints;
- the Session table preserves `TARGET` as its first column at every width, and
  detail views retain independent scrolling.

Footers show primary actions only; `?` always reveals the complete keymap, so
  narrow terminals lose discoverability, not capability.

Fullscreen detail modes, list selection, search, mouse/keyboard scrolling, and
confirmation behavior do not change when the layout reflows.

## Session-detail colors

The Calm Console palette is the default. Every Session detail text surface can
still be overridden under `[ui.session_detail.colors]`; existing custom
configuration continues to take precedence. See [Configuration](configuration.md).

## Terminal support

The UI uses Unicode box-drawing characters and RGB colors. Modern terminals
render the intended palette directly; terminals with a smaller color space may
approximate it. Native terminal selection and `Command+C` remain available in
Session detail views.
