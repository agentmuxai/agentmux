# Spec: taskbar/Dock presence for floating panes dragged to another monitor

## Context

Floating panes (torn-off panes in their own OS window) are currently built to
deliberately hide from the taskbar/Alt+Tab/Dock everywhere, all the time —
Windows uses `WS_POPUP | WS_EX_TOOLWINDOW` with no owner
(`agentmux-cef/src/floating_pane.rs:10,26`), explicitly documented as
"Taskbar/Alt+Tab hiding: `WS_EX_TOOLWINDOW`." That's the right default when a
floater sits on the same monitor as its main window — it reads as part of the
same app, not a separate top-level thing the user needs to switch to. It's
the wrong default once a floater has been dragged to a **different**
monitor: at that point it's spatially indistinguishable from any other
window on that monitor, and without a taskbar/Dock entry there's no way to
find it again except by looking at the right physical screen.

This spec covers **how** to give a floater real taskbar presence
conditionally — only when it's on a different monitor than its main window —
across the three platforms, and scopes what's actually buildable given where
each platform's multi-monitor support already stands in this codebase.

## What already exists (don't rebuild)

- **`set_taskbar_hidden(hwnd, hidden)`** (`agentmux-cef/src/commands/window_pool.rs:481-510`,
  Windows only) — an already-working, already-battle-tested primitive that
  toggles `WS_EX_TOOLWINDOW`/`WS_EX_APPWINDOW` on a live HWND, including the
  hide→restyle→show cycle Win32 requires for the shell to re-evaluate the
  taskbar entry. Currently only called for pool-window promotion. This is
  the exact mechanism this feature needs on Windows — no new Win32 taskbar
  API research required, just a new caller with a different trigger
  condition.
- **`get_monitor_work_area(px, py)` / `dpi_scale_at(px, py)`**
  (`agentmux-cef/src/app/monitor.rs`) — Windows-only, `MonitorFromPoint`-based
  per-monitor work-area and DPI lookup, already used for window placement
  math. **Not** currently used to identify *which* monitor a window is
  currently on as a stable, comparable value (see Design below) — that's new.
- **macOS and Linux equivalents of `get_monitor_work_area` are TODO stubs
  that unconditionally return `None`** (`monitor.rs:105-119`) — multi-monitor
  geometry isn't implemented on either platform yet. This bounds the scope of
  what's realistic here (see Non-goals).

## Platform research summary

**Windows.** Windows 11 already has native per-monitor taskbar support
end-to-end (Settings → Personalization → Taskbar → "Show my taskbar on all
displays", plus a policy for whether an app's button shows on "All taskbars"
or "Main taskbar and taskbar where window is open"). That's entirely the
shell's job once a window opts in — our only lever is the same one
`set_taskbar_hidden` already flips: `WS_EX_APPWINDOW` (has a taskbar
button, follows the OS's per-monitor policy automatically) vs.
`WS_EX_TOOLWINDOW` (never has one, anywhere). We don't need to reimplement
per-monitor placement logic; we need to decide *when* to flip the existing
switch.

**macOS.** There's no per-monitor Dock the way Windows has a per-monitor
taskbar — the Dock is a single instance that either stays on one display or
follows the cursor's display (governed by the user's own "Displays have
separate Spaces" Mission Control setting, which this app doesn't and
shouldn't touch). The equivalent lever here isn't "which monitor's Dock" but
"does this window show up in Cmd+Tab / Mission Control's window list at
all" — controlled by whether the floater is built as an `NSPanel`
(non-activating, normally excluded from both) vs. a regular `NSWindow`, or
by toggling `NSWindow.collectionBehavior` /
`NSApplication.ActivationPolicy` on the existing window. Needs a
codebase-specific check (not covered by this pass) of how CEF Views exposes
that toggle on macOS, since research surfaced the toolkit-level lever
(window type / activation policy) but not this app's existing wrapper for it.

**Linux.** No standardized per-monitor taskbar concept exists across desktop
environments the way it does on Windows — taskbar/panel behavior is
compositor- and DE-specific (GNOME's default shell doesn't have a persistent
taskbar at all; KDE/XFCE panels are user-configured per-screen or not).
The portable lever is the EWMH `_NET_WM_WINDOW_TYPE` hint (set to `UTILITY`
for a floater, `NORMAL` if it should behave like a full top-level app
window) plus the `_NET_WM_STATE_SKIP_TASKBAR`/`SKIP_PAGER` state hints —
toggling those is the X11 equivalent of `set_taskbar_hidden`. This only
applies to X11 (including XWayland — Chromium/CEF apps commonly run as an
XWayland client even under a Wayland session, so the EWMH hints likely still
apply via translation, but this needs live verification on this app's actual
Linux runtime rather than assuming). Native-Wayland taskbar-equivalent
surfaces (`wlr-layer-shell`) are a different, compositor-specific API family
and are out of scope here.

## Design — Windows (the only platform this phase implements end-to-end)

1. **Stable per-window monitor identity.** Add a helper alongside
   `get_monitor_work_area` that returns the `HMONITOR` handle for a given
   HWND (`MonitorFromWindow`, not `MonitorFromPoint` — a window-based lookup
   is more direct than re-deriving from a point sample, and `HMONITOR`
   handles are stable identifiers for comparison across calls, per Win32
   semantics). `MonitorFromWindow(main_hwnd)` vs.
   `MonitorFromWindow(floater_hwnd)` — different handles means "on a
   different monitor," which is the entire trigger condition.
2. **When to check.** Hook into the same places that already move a floater:
   the JS-driven drag-move IPC (`set_window_rect`/`set_window_position`,
   `agentmux-cef/src/commands/window/motion.rs`) and the Windows native move
   loop (`Win32BeginMoveTask`, `agentmux-cef/src/ui_tasks/drag.rs`) — both
   already run on every meaningful position update, so this is a cheap
   comparison added to an existing hot path, not a new poll loop.
3. **Debounce the toggle**, not just the check — `set_taskbar_hidden`'s own
   doc comment notes it does a hide→restyle→show cycle, which will flicker
   the window if called on every drag-move tick while straddling a monitor
   boundary. Gate on the same dwell-style pattern already used for redock
   hover (`REDOCK_DWELL_MS`, `frontend/app/workspace/floating-pane-constants.ts`)
   rather than inventing a new debounce primitive — settle on "monitor
   changed and stayed changed for N ms" before flipping the taskbar style,
   and flip back immediately (no dwell needed) once it's confirmed back on
   the main window's monitor, since the failure mode of "removed the
   taskbar entry a little early" is much less disruptive than "added and
   immediately removed it" flicker.
4. **On drop / drag end**, do one final authoritative check (not just trust
   the last debounced state) — mirroring how `tryRedockAtCursor` re-resolves
   the target at mouseup rather than trusting the last hover event.

## Non-goals (this phase)

- **macOS and Linux implementations.** Both require groundwork this codebase
  doesn't have yet (`get_monitor_work_area`'s TODO stubs), and each platform's
  actual lever needs its own short investigation (this app's CEF-Views
  wrapper for macOS activation-policy/collection-behavior; live verification
  of EWMH hint propagation through this app's Linux/XWayland runtime) rather
  than being derivable from this pass's Windows-focused research. Track as
  explicit follow-ups once each platform's basic multi-monitor work-area
  detection lands (which is itself already a known gap, not new scope this
  spec introduces).
- **Native Wayland support** (`wlr-layer-shell` or equivalent) — out of scope
  entirely; XWayland-via-EWMH is the only Linux path this spec considers.
- **User-configurable policy** (e.g., "always show floaters in taskbar," "never
  show them") — start with the single automatic rule (different monitor from
  main → visible); a settings toggle can follow if the automatic behavior
  turns out to need an escape hatch, but isn't assumed necessary up front.

## Sequencing note

`docs/specs/SPEC_REDOCK_FRAMEWORK_HARDENING_2026_07_27.md`'s Phase 3 (adopt
the unowned-floater model, re-anchor taskbar visibility via an explicit,
owner-independent `WS_EX_TOOLWINDOW` flag rather than implicit owned-popup
behavior) should land **before** this spec's implementation phase. Once
taskbar visibility is already driven by an explicit flag for other reasons,
adding "flip it based on monitor" is a targeted policy change; doing it first
means fighting the current owner-coupled model this feature doesn't actually
need to touch otherwise.

## Verification (Windows phase)

- Manual: two-monitor setup, drag a floater from the main window's monitor
  to the other — taskbar entry appears (respecting whatever the user's own
  Windows per-monitor taskbar setting is; we don't override that policy,
  only whether the window participates in it at all); drag back — entry
  disappears without leaving a stale/ghost taskbar button.
- Manual: rapid back-and-forth dragging across the monitor boundary doesn't
  flicker the taskbar entry (dwell gate holds).
- Manual: closing a floater while it has a taskbar entry doesn't leave
  anything behind (existing window-close path already handles HWND
  teardown; this only adds a style flag, no new lifecycle to clean up).

## Files (expected)

- `agentmux-cef/src/app/monitor.rs` — new `monitor_for_window(hwnd) -> Option<HMONITOR>` (Windows)
- `agentmux-cef/src/commands/window_pool.rs` — reuse `set_taskbar_hidden`, no changes needed to the function itself
- `agentmux-cef/src/commands/window/motion.rs`, `agentmux-cef/src/ui_tasks/drag.rs` — hook the monitor-change check into existing move-update paths
- `frontend/app/workspace/floating-pane-constants.ts` — new dwell constant if the redock dwell constant isn't reused directly
