---
name: ratatui-feature
description: Implement or change a Rust terminal UI built with Ratatui. Use for screens, widgets, dialogs, layouts, navigation, key handling, selections, rendering, and other Ratatui behavior.
---

# Implement a Ratatui feature

1. Inspect existing state ownership, event loop, action mapping, rendering modules, terminal setup, and tests. Keep the project's architecture unless a change requires otherwise.
2. Model UI states explicitly. For mutually exclusive modes, prefer an enum over overlapping flags. Keep selected index, focus, scroll position, dialogs, and active view under a clear owner.
3. Where it fits, translate `Terminal Event -> Action -> State Update -> Render`. Keep complex behavior out of key-event matching. Keep rendering dependent on current state and layout; perform filesystem, network, and background work outside drawing code.
4. Make navigation safe for empty and one-item collections, boundaries, and selections invalidated by filtering or removal. Tolerate narrow, short, and resized terminals; avoid dimension underflow.
5. Preserve reliable restoration of raw mode, alternate screen, and cursor on normal exit, errors, and early return. Avoid printing diagnostics into the active TUI.
6. Test state transitions, key-to-action mapping, and navigation without a terminal. For meaningful visual behavior, use Ratatui `TestBackend` or buffer assertions; cover selected and empty states and small layouts where relevant. Prefer semantic assertions over broad snapshots.
7. Run relevant tests and the `rust-verify` workflow if available. Inspect the diff for unrelated UI changes.
