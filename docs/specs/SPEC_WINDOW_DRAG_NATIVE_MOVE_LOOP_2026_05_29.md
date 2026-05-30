---
title: Window drag — replace per-mousemove IPC with the OS native move loop
status: Draft / Proposed
date: 2026-05-29
author: AgentX
front: window-drag (UX-latency umbrella #1161)
related:
  - "#1161 Typing/input-responsiveness umbrella (the broader 'remove UX latency' effort)"
  - "#1097 SetWindowRgn airspace cost (sibling native-path latency)"
  - "PR #1178 perf(airspace): region-cache (sibling, in-flight)"
  - "docs/specs/SPEC_WINDOW_DRAG_DPI_FIX_2026-05-13.md (the DPI hand-correction this spec deletes)"
  - "Codex PR #734 (rounds 2-4: the race-guarding this spec deletes)"
  - "docs/analysis/ANALYSIS_UX_LATENCY_THREE_FRONTS_2026_05_29.md (findings this spec implements)"
---

# Window drag → OS native move loop (Windows)

## 1. Problem

On Windows, dragging the main window (or a floating-pane window) by its title bar
**lags the cursor**, and the lag gets visibly worse at non-100% display scale.

The cost is **entirely self-inflicted by the frontend**. `useWindowDrag.win32.ts`
drives the *whole* drag in JavaScript:

- **mousedown** in a `data-drag-region` → `await get_window_position` (one IPC
  round-trip *before the drag even arms*).
- **every mousemove (~120 Hz)** → compute `target = initWin + delta * devicePixelRatio`,
  then `set_window_position` over HTTP-IPC. The window's new position is **gated on
  that round-trip completing** (`useWindowDrag.win32.ts:143-161` → `:126-142` →
  `ipc.ts:51` fetch → `motion.rs:92` `SetWindowPos`).

Each move pays: localhost `fetch` (connection + HTTP framing + JSON) → tokio worker
dispatch → `GetWindowRect` + `SetWindowPos` → JSON response → promise resolution.
A one-in-flight coalescer (`:124-142`) drops intermediate moves, so under any IPC
jitter the window updates at the **round-trip rate, not the input rate** — the lag.

It also carries a whole tail of accidental complexity that exists *only* to paper
over the JS-driven approach:

- DPI hand-correction (`* devicePixelRatio`, `:103-105`, `:157-159`) — the source
  of the documented ~20% lag at Win11 125% scale (`SPEC_WINDOW_DRAG_DPI_FIX_2026-05-13.md`).
- Per-mousedown sequence token, catch-up move, one-in-flight coalescer — three
  rounds of race-guarding from Codex PR #734 (`:43-65`, `:113-142`, `:163-175`).

## 2. The key realization

There is **nothing to "freeze" during a window move.** (This spec supersedes the
"freeze the DOM during drag" idea — see the verdict in
`ANALYSIS_UX_LATENCY_THREE_FRONTS_2026_05_29.md §Drag`.)

A *pure move* changes the window's position, not its size:

- The floating-pane wndproc handles `WM_SIZE` but has **no `WM_MOVE` handler**
  (`floating_pane.rs`), so child pane HWNDs follow the parent automatically — **zero
  reflow**.
- `set_pane_overlay_clip` (the airspace `SetWindowRgn` path) is **not** triggered by
  a move — its only frontend trigger sweeps on DOM `resize`/`scroll`
  (`pane-overlay-auto.ts`), and a window move changes no inner content size.
- The OS already composites the last frame of every child HWND and moves them as one.

So the renderer should not be involved in a move *at all*. The fix is not to suppress
renderer work — it's to **stop generating it**.

## 3. The capability already exists (and is unused)

The Win32 host **already implements the native move loop** — it's just never called
from the Windows frontend:

```rust
// agentmux-cef/src/commands/window/motion.rs:135  start_window_drag()
#[cfg(target_os = "windows")]
unsafe {
    let hwnd = find_own_top_level_window();
    if !hwnd.is_null() {
        ReleaseCapture();
        SendMessageW(hwnd, WM_NCLBUTTONDOWN, 2 /* HTCAPTION */, 0);
        return Ok(serde_json::Value::Null);
    }
}
```

`WM_NCLBUTTONDOWN` + `HTCAPTION` makes `DefWindowProc` run the **OS modal move loop**:
the OS moves the frame and all child HWNDs following the cursor until the user releases
the button — **zero further IPC, zero renderer involvement**, and Aero Snap / multi-
monitor / shake-to-minimize all work for free.

**The Linux/macOS frontend already uses this pattern** (`useWindowDrag.linux.ts`): on
mousedown it arms, and on a 4px threshold-crossing mousemove it fires **one**
fire-and-forget `start_window_drag` IPC; the compositor takes over. Host side, the
non-Windows arm routes to `CefWindow::BeginWindowDrag()` (a CEF patch — `xdg_toplevel.move`
on Wayland / `_NET_WM_MOVERESIZE` on X11).

**Only the Windows frontend opts out**, and the file says why
(`useWindowDrag.win32.ts:7`):

> `WM_NCLBUTTONDOWN doesn't work because the async IPC roundtrip loses mouse state.`

## 4. Root cause of "loses mouse state"

The `WM_NCLBUTTONDOWN`/`HTCAPTION` modal loop only engages if the **left button is
still physically down and capture is consistent** when `DefWindowProc` processes the
message. Two things break that in the current shape:

1. **CEF already captured the pointer.** The renderer takes pointer capture on the
   web-content mousedown. When the raw `SendMessageW(WM_NCLBUTTONDOWN)` arrives, the
   move loop's hit-test/capture handoff conflicts with CEF's capture and the loop
   exits immediately. `ReleaseCapture()` (already present) helps but isn't integrated
   with CEF's input pipeline.
2. **Async IPC latency.** If `start_window_drag` is *awaited* (or simply slow), the
   `WM_NCLBUTTONDOWN` lands after the button state / capture has diverged from the
   press.

The Linux/macOS path avoids both by initiating the OS drag **from inside CEF**
(`BeginWindowDrag()`), which preserves capture and mouse state. Windows has no
equivalent wired — it uses the raw `SendMessage` hack, which is exactly why it
"loses mouse state."

## 5. Proposal

Delete the per-mousemove JS drag on Windows and hand the move to the OS, mirroring
the Linux path. Two implementation options, recommended in order:

### Option B (cheap spike — try first)
Point the existing Win32 `start_window_drag` at the frontend with correct timing:

- **Frontend (`useWindowDrag.win32.ts`):** rewrite to mirror `useWindowDrag.linux.ts`:
  - mousedown in `data-drag-region` → arm (record press point), **do not** `preventDefault`,
    **do not** `await get_window_position`.
  - mousemove crossing a 4px threshold while the button is held → fire **one**
    fire-and-forget `invokeCommand("start_window_drag", { label })`, then stop tracking.
  - mouseup → disarm. dblclick → `maximize_window` (unchanged).
  - **Delete** `set_window_position`/`sendPos`, the `* devicePixelRatio` math, the
    sequence token, catch-up, and one-in-flight coalescer — all unnecessary once the
    OS owns the move.
- **Host:** unchanged (`start_window_drag` already does `ReleaseCapture()` +
  `SendMessageW(WM_NCLBUTTONDOWN, HTCAPTION)`).

The bet: the historical "loses mouse state" came from **awaiting** the IPC and/or the
`get_window_position` round-trip delaying arm — not from a fundamental Win32
limitation. Firing one fire-and-forget message on the threshold-crossing mousemove
(button still held) may now just work. **Spike on Win11 @ 125% scale before committing.**

### Option A (correct fix — fallback if B loses state)
Port `CefWindow::BeginWindowDrag()` to Windows and route through it:

- Confirm whether the CEF patch that added `BeginWindowDrag` to the `CefWindow` API
  (used by Linux/mac) already has a Windows/Aura implementation; if not, add one
  (on Windows it ultimately still issues the `WM_NCLBUTTONDOWN` move, but **from
  inside CEF's input handling**, so capture/mouse-state stay consistent).
- Change the Win32 arm of `start_window_drag` (`motion.rs:136`) to call
  `BeginWindowDrag()` instead of the raw `ReleaseCapture` + `SendMessageW`.
- Frontend change is identical to Option B.

Option A is the durable, cross-platform-consistent answer; Option B is a 1-hour
experiment that might make A unnecessary.

## 6. What we gain / what to watch

**Gain**
- Window tracks the cursor at input rate, **insensitive to IPC jitter**.
- **Deletes** the DPI hand-correction (`SPEC_WINDOW_DRAG_DPI_FIX`) and its 125%-scale
  bug class — the OS moves in physical px natively.
- **Deletes** ~120 lines of race-guarding (PR #734 rounds 2-4).
- Aero Snap, snap-assist, multi-monitor DPI transitions, shake-to-minimize: free.

**Watch (open questions / must-verify)**
- **Tab tear-off** (drag a *tab* below the bar to spawn a new window) is a separate
  gesture on a separate element; confirm it does **not** share the title-bar
  `data-drag-region` path and is unaffected.
- **Floating-pane windows** must drag themselves, not "main." The native loop uses
  `find_own_top_level_window()` (no label) — verify it resolves the correct HWND for
  an owned floater (the JS path used `ownWindowLabel()` + `resolve_window_hwnd` for
  exactly this reason — `motion.rs:128-134`, `useWindowDrag.win32.ts:24-37`).
- **Modal loop blocks the UI thread** until drop (standard Windows behavior). Confirm
  no timer/IPC starvation issue during a long drag (renderer is on its own process/
  threads, so this should be invisible).
- **Right-click / context menu** on the title bar must still fire (Linux kept the
  region `HTCLIENT` precisely for this — `useWindowDrag.linux.ts:13`). Verify on Win32.

## 7. Measurement (gate for merge)

Extend the **#1176 input-latency bench harness** with a **drag harness** that measures
*cursor→window lag*, not handler time:

- Script a constant-velocity drag; each frame sample the window rect (the WRR
  `LOCATIONCHANGE` stream, `win_event.rs:352`, is a ready position source) vs. the
  synthetic cursor.
- **Before:** lag tracks IPC round-trip rate and **grows under injected localhost
  jitter**.
- **After (success):** lag is ~one frame and **insensitive to injected IPC jitter** —
  the proof the move left the IPC path.
- Assert `set_window_position` call-count during a drag drops to **0**, and exactly
  **one** `start_window_drag` fires per drag (on the threshold crossing).
- Re-run at **100% / 125% / 150%** Win11 scale — expect the 125% lag to *disappear*
  (DPI math removed), not regress.

Freebie shipped alongside: demote the per-`LOCATIONCHANGE` `tracing::info!`
(`win_event.rs:236-240`, fires 5-20×/s during a drag) to `debug!`.

## 8. Rollout

1. Land the `tracing` demotion (independent, trivial).
2. **Spike Option B** behind a runtime flag (`AGENTMUX_NATIVE_WINDOW_DRAG=1` or a
   settings toggle) so the JS path remains the default fallback during validation.
3. Run the drag harness + manual matrix (tear-off, floaters, context menu, snap,
   multi-monitor @ 3 scales).
4. If B holds mouse state → flip the flag default to native, delete the JS path next
   release. If B loses state → implement Option A, same validation.

## 9. Acceptance criteria

- [ ] Drag lag is flat and jitter-insensitive in the harness; `set_window_position`
      count = 0 during drag.
- [ ] No regression at 125%/150% scale (expect improvement).
- [ ] Tab tear-off, floating-pane drag-self, title-bar context menu, dblclick-maximize
      all still work on Windows.
- [ ] JS drag path remains behind a flag for one release as fallback.
