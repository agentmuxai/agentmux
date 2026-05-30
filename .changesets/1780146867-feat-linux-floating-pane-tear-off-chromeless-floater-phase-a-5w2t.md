---
type: patch
---

feat(linux): floating-pane tear-off — chromeless floater (Phase A, mirrors macOS #1182)

On Linux, tearing a pane off used to produce a full workspace window
(tab bar + widget bar). Now it produces a chromeless floating window —
"just the pane" — matching Windows and macOS (the latter shipped this
in #1182).

One-file frontend change in
`frontend/app/drag/CrossWindowDragMonitor.linux.tsx`:

- Pane branch in `performTearOff` now calls
  `open_floating_pane_window` (chromeless), mirroring the win32 and
  darwin siblings. Imports `measureSourcePaneSize` from the shared
  helper and uses it to size the floater at the source pane's
  rendered size (not the parent window's outer size). IPC-first /
  mutate-on-success ordering matches the win32 reference (Reagent
  P1 on #1073).
- Tab branch unchanged. Tab tear-off still spawns a full top-level
  instance with its own taskbar entry.

No backend work: #1182 already widened the
`agentmux-cef/src/commands/floating_pane.rs` non-Windows branch
from "not yet implemented" to a real implementation that runs
identically on Linux and macOS. Secondary windows on Linux are
already frameless CEF Views windows (`window_create_top_level
frameless=true`), and the chromeless renderer
(`<FloatingPaneWorkspace>`) is purely a function of the
`?floatingPaneId=` URL param — both platform-agnostic.

Phase A scope only. Owned-window lifecycle (Gtk `transient-for` +
`skip-taskbar-hint` + `destroy-with-parent`), JS-driven header drag,
and floater redock are Phase B+ (tracked separately).

Spec: docs/specs/SPEC_LINUX_FLOATING_PANE_TEAROFF_2026_05_30.md
