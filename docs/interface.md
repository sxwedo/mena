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

## Responsive layout

The interface adapts from the frame size on every redraw:

- below 100 columns, the Skill list/preview and MCP list/details panes stack
  vertically;
- below 88 columns, the launcher hides its lower-priority session-context
  column;
- below 96 columns, footers collapse to the primary key hints;
- the Session table preserves `TARGET` as its first column at every width, and
  detail views retain independent scrolling.

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
