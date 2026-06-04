---
type: patch
---

fix(cef): pin Cargo to CEF 146 until 7778 drag/right-click/transparency patches are forward-ported

PR #1221 (CEF 146 → 148 source bump) inadvertently broke
`build:host:linux` because the agentmux patches that add
`_cef_window_t::begin_window_drag` (the source code addition behind
`StartWindowDragTask` for native Wayland/X11 window drag) only exist on
the `agentmuxai/cef@agentmux/7680-drag-rightclick-and-transparency`
branch — they have not been forward-ported to the
`agentmuxai/cef@agentmux/7778-…` branch yet.

Consequences observed locally:

- `cargo build --release -p agentmux-cef --features patched-libcef`
  fails at `ui_tasks.rs:215` with
  `error[E0609]: no field begin_window_drag on type _cef_window_t`,
  because the cef-dll-sys 148 binding lacks the field.
- To get a green build, the Linux Taskfile path has been silently
  building **without** `--features patched-libcef`, which compiles the
  `#[cfg(not(feature = "patched-libcef"))]` no-op branch — every
  `start_window_drag` IPC then warns "patched-libcef feature disabled
  — native drag is a no-op." Title-bar drag silently does nothing on
  every Linux build since the bump, regardless of ozone platform.

This pin restores the previous state: CEF 146 with the patched
binding, `--features patched-libcef` compiles, and the existing
patched `libcef.so` (built from
`agentmuxai/cef@agentmux/7680-drag-rightclick-and-transparency` via
`scripts/resolve-cef-runtime.sh`) is the matching runtime.

This is **explicitly temporary**. The proper fix is two PRs:

1. New branch `agentmuxai/cef@agentmux/7778-drag-rightclick-and-transparency`
   cherry-picking these commits onto upstream `7778`:
   - `af485ed2` views: Add `CefWindow::BeginWindowDrag()`
   - `010f616f` views: PR review — pass actual cursor screen point
     (the X11 fix; without it `_NET_WM_MOVERESIZE` ignores the
     request)
   - `130af663` views: Annotate API version
   - `41802fe6` Patch A: right-clicks on HTCAPTION fall through to renderer
   - `b921ffe1` Support transparent window in Views framework
   - The five `views: …transparency cascade…` follow-ups
2. `cef-dll-sys` 148 binding adds the `begin_window_drag` field.

Once both land and a CEF 148 `libcef.so` is rebuilt and bundled,
Cargo can move back to `"148"`.

macOS and Windows tasks are unaffected: macOS builds CEF 148 from
`agentmuxai/cef@agentmux/7778-process-requirement` (which has only the
macOS-26 patch, no drag patch); Windows doesn't use
`--features patched-libcef` (native drag goes through a different
Windows-specific path).

Local validation: with this pin, `task build:host` succeeds, the
resulting host binary contains the `BeginWindowDrag returned` info
string (not the disabled-feature warn string), and title-bar drag
works on both Wayland and X11 ozone modes against the existing CEF
146 `libcef.so`.
