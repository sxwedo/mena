# Native macOS app

The desktop app uses SwiftUI and AppKit over mena's existing Rust catalogs.
It requires macOS 14 or later. Building requires Xcode 26 / Swift 6.2 or
newer, Rust 1.96.1, and the Xcode command-line tools. The first build fetches
UniFFI and SwiftTerm; neither adds an account, daemon, or remote store.

## Build and run

```sh
mise run gui                     # build and open
mise run gui-build               # build without opening
mise run gui-release             # optimized app
open target/macos/Mena.app
```

The app is built at `target/macos/Mena.app`. It includes its Rust library
and receives an ad-hoc signature for local development. You can copy the
bundle into `/Applications` or `~/Applications`, then use `mena gui` to
open it. The CLI install does not automatically install the app. Public
distribution additionally needs Developer ID signing and notarization.

The macOS release script disables Rust's debug-info stripping: the pinned
toolchain can otherwise produce a Mach-O string pool rejected by Xcode 27's
linker ([upstream issue](https://github.com/rust-lang/rust/issues/157750)).
The library's install name is set at link time.

## Workspace

- The three columns contain project/navigation, the current catalog, and
  details. Session filters include provider, project, title, target, and
  bounded message-content search. New searches cancel obsolete work and keep
  the selected detail aligned with the filtered catalog.
- SubAgents are hidden initially. A persistent switch at the bottom of
  the session list reveals them; badges distinguish SubAgents and native
  branches. Session information links to a known parent when available.
- Select a message by clicking its card or using the arrow keys while the
  transcript is focused. A mint marker, outline, and fork-origin badge indicate
  the selection. Search hits use a separate yellow highlight.
- Session details initially show user and assistant messages. The menu can
  reveal tool and system records. Titles are mena-owned overlays; renaming
  never changes provider transcripts.
- Session messages and Markdown resource previews render headings, nested
  lists, task lists, quotes, tables, inline formatting, and fenced code with
  copy actions. The menu switches to source text and adjusts reading size;
  Settings remembers the font size. Embedded images show their descriptions
  without fetching remote content. The footer jumps to the first/latest message.
- Projects appear in recent-session order, initially limited to eight entries
  with an option to show all. Session rows keep technical IDs in the information
  panel. Long user messages collapse until expanded.
- Resource pages prioritize readable content. Source details are expandable;
  Skill frontmatter stays available in source mode instead of becoming a heading.
  MCP pages show connection and probe summaries first; raw metadata is expandable.
- Skills, MCP registrations, and memories use the same catalogs and bounded
  reads as the CLI. Select a project to include its project-level resources.
  MCP discovery is static; only the explicit **Probe service** action can
  start a configured server or contact its endpoint. Probes return metadata
  only, and their results remain attached to their originating registration.
- The running-process view only links a process to a session when current
  provider-native evidence confirms the association.

The sidebar and Settings support system, light, and dark appearance. Refresh
the list with `⌘R`; returning to the workspace also refreshes its catalog.
`⌘K` focuses catalog search, `⌘F` searches messages, `⌘⇧F` opens message fork,
and `⌘Return` continues the selected session. `⌘⇧L` toggles focused reading,
hiding the two navigation columns. The app icon and sidebar use the
README artwork; the bundled PNG is its static first frame. Icon framing focuses
on the head and shoulders so the portrait fills the available space.

## Export Markdown

Use the detail header's export button or `⌘⇧E`, then choose a folder.
The export follows the current message scope: conversation messages by default,
or complete history when **Show tool and system records** is enabled.
It re-reads the native session through the catalog and reuses the CLI's Markdown
exporter. Files are private (`0600` on Unix), and existing files are never
replaced. A completion notice offers **Show in Finder**.

## Start, continue, and fork

Use **New chat** or `⌘N` to select an installed built-in or configured custom
Agent and a project directory. The app reuses the CLI's native fresh-launch
argv and opens a terminal. The Agent creates its own session after launch;
no placeholder conversation is added to the catalog.

Continue opens a separate terminal window using SwiftTerm and the native
Agent executable. The program, argv, and working directory are passed
separately, without a shell command string. Standard user and Homebrew
binary directories supplement Finder's sparse PATH. Closing a running
terminal or quitting the app requires confirmation before stopping its
owned processes.

The fork sheet distinguishes two methods:

1. **Native fork** is available at persisted completed-turn boundaries
   supported by the provider, currently Codex. It creates an independent
   native session through that boundary and saves its display title.
2. **New chat with context** transfers the normalized transcript prefix
   through the selected message into a fresh installed Claude, Codex, or
   Oh My Pi process. Tool execution state and native session state are not
   inherited. Its name labels the terminal window; the target Agent creates
   its own native session after launch.

Both routes re-read the history and reject a changed selection or prefix.
Later messages are excluded. Context handoffs are private temporary files,
owned by the prepared launch and retained while the terminal process runs.
The original session stays intact.

## Implementation and verification

`src/desktop.rs` is the optional `desktop` feature's typed interface.
`src/desktop/model.rs` defines the values crossing the interface;
`resources.rs` projects redacted catalog metadata; `launch.rs` owns prepared
argv and context-file lifetime. Provider storage logic stays in its existing
adapters. UniFFI generates the Swift bindings into ignored
`macos/Generated/`; do not edit generated files.

`macos/Sources/Mena/WorkspaceModel.swift` owns UI selection, request
generations, and preferences. Blocking Rust calls run outside the main
actor, and obsolete replies cannot replace the current selection. SwiftUI
views are split by workspace, session, resources, and action sheets;
`TerminalWindows.swift` owns terminal-process lifetime. `MessageFormatting.swift`
uses Foundation's Markdown parser and projects its block intents into native
SwiftUI layout; it does not add a parser dependency or a web view.

```sh
mise run verify      # existing CLI gates
mise run gui-verify  # CLI + desktop Rust checks + native tests/format + debug/release builds
```

Native model tests use temporary provider stores and isolated preferences.
`MENA_GUI_HOME` can point the app to a fixture home for manual UI testing;
it uses separate desktop preferences, and does not change `HOME` or relocate
provider commands' own configuration.
The app supports `--workspace /absolute/project/path` when launched directly.

Native window checks complement the model tests: search, change appearance,
submit both fork methods, start a fresh Agent, and explicitly probe a fixture MCP
server. Toggle focused reading and verify that `⌘K` restores and focuses catalog
search. Resize to the minimum width and confirm the transcript still wraps.
After these actions, idle CPU should settle and the app must remain responsive.
These checks exercise AppKit layout and progress indicators that model tests do
not instantiate. Native controls use resolved tints to avoid dynamic-color
constraint loops.

This version supports browsing, renaming, resuming, message forks, and Markdown
export. Trash/restore and configuration mutations remain available through
the CLI/TUI.
