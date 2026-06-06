# Retro: Floater drag state bugs — PR #1276 / #1280

**Date:** 2026-06-05
**PRs:** #1279 (spec), #1280 (implementation)
**Symptom:** Redock broken on Windows; dragging state permanently stuck on macOS/Linux; unexpected floater closes when maximising; stale drop-target overlays after Esc.

---

## What happened

PR #1276 replaced the JS-driven `get/set_window_position` polling loop with a host-side `Win32BeginMoveTask` manual capture loop on Windows and `CefWindow::BeginWindowDrag` on macOS/Linux. The drag smoothness goal was met. The redock, hover, and state-cleanup correctness goals were not.

Seven bugs shipped and required three review cycles to surface:

### BUG-1 (P0) — `SendMessageW(h, WM_LBUTTONUP, 0, 0)` — wrong cursor coordinates

`Win32BeginMoveTask`'s "safety-net" synthesis path sent `lParam=0`, which encodes client coordinates `(0, 0)` — the floater's top-left corner — not the actual cursor position. The comment in `floating-pane-workspace.tsx` said *"the host dispatches WM_LBUTTONUP … so this fires at the actual release cursor position"* — this was factually incorrect for the synthesis path. The DOM `MouseEvent.screenX/Y` resolved to the floater's origin, so `resolve_window_at_cursor` hit-tested the floater itself, the `exclude_label` filter removed it, and the result was `{label: null}` — every redock silently no-oped.

**Fix:** `GetCursorPos` + `ScreenToClient` before `SendMessageW`, encode real client coords in `lParam`.

### BUG-2 (P0) — `dragging` permanently true on macOS/Linux

`BeginWindowDrag` hands the gesture to the OS compositor, which absorbs `mouseup` — no DOM `mouseup` is re-delivered to the renderer after the drag completes. The Win32 path was explicitly engineered to synthesize `WM_LBUTTONUP` (PR #1181) so the renderer sees its balancing up; no equivalent was built for the `BeginWindowDrag` path. `dragging` stuck `true` after every drag, causing `update_floating_redock_hover` to fire on every subsequent mouse move in the window for the rest of the session.

**Fix:** `StartWindowDragTask::execute` now emits `window_drag_ended` after `BeginWindowDrag` returns. `Win32BeginMoveTask` also emits `window_drag_ended` after `ReleaseCapture`. The renderer resets `dragging = false` in the `window_drag_ended` handler.

### BUG-3 (P1) — `dragging` guard used time instead of motion

The 200ms press-duration guard was a proxy for "did the window actually move." A slow deliberate hold (>200ms, zero displacement) passed the guard and called `tryRedockAtCursor`. This caused unexpected redocks when hovering the header before maximising — the hold satisfied the 200ms threshold.

**Fix:** Replaced the time guard with a split-platform approach. On Windows: the host includes `cursor_x/cursor_y` in `window_drag_ended` (physical px from `GetCursorPos`) and `moves > 0` gates `tryRedockAtCursor` — the renderer's `window_drag_ended` handler drives the redock directly, eliminating the async race between DOM mouseup and the event. On macOS/Linux: the `onMouseMove` handler sets `hasMoved = true` on first pixel move; `onMouseUp` calls `tryRedockAtCursor` directly only when `hasMoved` is true, preventing plain header clicks from false-redocking.

### BUG-4 (P1) — `window_drag_cancelled` broadcast with no label guard

`emit_event_to_top_level_windows` broadcasts to all top-level browsers. The renderer handler cleared `dragging = false` on any `window_drag_cancelled` event without checking whether the `label` field matched its own window label. An Esc-cancel on floater-A silently cleared `dragging` on floater-B, suppressing any concurrent redock on B.

**Fix:** Added `if (!ev.label || ev.label === label)` guard to all three new `listenEvent` handlers.

### BUG-5 (P1) — `listenEvent` unlisten handle is racy with `onCleanup`

`listenEvent` returns a `Promise`. `onCleanup` is synchronous. If the component unmounted before the microtask queue delivered the resolved Promise, the unlisten handle was still `null` when `onCleanup` ran — the `window` CustomEvent listener leaked permanently. Each tearoff+remount+early-unmount cycle accumulated a new leaked listener.

**Fix:** Introduced a `safeListenEvent` helper that holds a `cleaned` sentinel (set to `true` in `onCleanup` as its first action). The `.then()` body checks `if (cleaned) { unlisten(); }` before storing the handle.

### BUG-6 (P1) — `mouseup` registered bubble-phase, `mousedown` capture-phase

`mousedown` was consciously registered as capture-phase to pre-empt pragmatic-dnd. `mouseup` was registered as bubble-phase (no third argument). Any child element calling `stopPropagation` on `mouseup` left `dragging = true` indefinitely — same runaway hover-emit storm as BUG-2.

**Fix:** Changed to `document.addEventListener("mouseup", onMouseUp, true)` (capture-phase).

### BUG-7 (P1) — Auto-close watcher races in-flight `RedockFloatingPane`

The backend broadcasts the `Tab` WaveObj update (empty `blockids`) before the `RedockFloatingPane` RPC response is received by the floater's `tryRedockAtCursor` await. The `createEffect` auto-close watcher reacted to the broadcast immediately — `hadBlocks` was already `true` from mount — and called `closeWindowByLabel` while the redock was still mid-flight, destroying the floater window before the redock completed.

**Fix:** `redockInProgress` flag (component scope, shared between `createEffect` and `onMount`). Set before `WorkspaceService.RedockFloatingPane`, cleared in `finally`. `createEffect` now checks `hadBlocks && !redockInProgress` before closing.

---

## Root cause themes

**1. Synthesized DOM input events must carry real cursor state.**
`SendMessageW` for `WM_LBUTTONUP` requires `MAKELPARAM(clientX, clientY)` with the actual cursor position. `lParam=0` is only correct if the cursor is genuinely at client `(0,0)`. Any time a pointer Win32 message is synthesized, the lParam must be populated from `GetCursorPos` + `ScreenToClient`. This is a recurring footgun: it appeared in SC_MOVE handshake attempts before PR #1181 and now again in the `WM_LBUTTONUP` synthesis path.

**2. Platform event lifecycle must be verified end-to-end before shipping.**
`BeginWindowDrag` (macOS/Linux) and `Win32BeginMoveTask` (Windows) differ fundamentally in whether they re-deliver `mouseup` to the web renderer. The spec marked this as "F3 — verify pending" but the PR was shipped with F3 open. Cross-platform drag implementations must verify the complete event lifecycle (`mousedown`, `mousemove`, `mouseup`) on each target OS before the renderer-side state machine is simplified to assume it.

**3. Async unlisten registration is structurally racy with synchronous `onCleanup`.**
The pattern `listenEvent(...).then(u => { ref = u; })` is inherently unsafe because `onCleanup` runs synchronously on component teardown while `.then()` delivers asynchronously. The `safeListenEvent` helper with a `cleaned` sentinel is the correct pattern and should be used for all IPC listener registrations in async-unmount-sensitive components.

**4. Broadcast IPC events require recipient-side label filtering.**
`emit_event_to_top_level_windows` is a process-wide broadcast. Any event that carries per-instance semantics (`window_drag_cancelled`, `window_drag_ended`) must be filtered by the recipient against its own `windowLabel`. The spec noted "minimal: `{label}`" but did not specify that recipients must match — this is now explicit.

**5. Proxy-based guards are fragile; prefer host-reported signals.**
Time duration was used as a proxy for window movement. Duration does not imply displacement — a slow touchpad hold satisfies any reasonable time threshold. On Windows the host now surfaces `moves > 0` via `window_drag_ended { cursor_x, cursor_y }` so the renderer gates redock on actual motion. On non-Windows the DOM `onMouseMove`-based `hasMoved` flag serves the same purpose.

**6. Reactive close watchers need in-flight guards for async RPC flows.**
The `createEffect` pattern that reacts to WaveObj broadcasts is the right teardown mechanism. It becomes dangerous when the same broadcast that triggers close is emitted mid-way through an async operation that the renderer is awaiting. Any component that calls a long-running RPC and then auto-closes on a side-effect of that RPC must guard the auto-close path against the in-flight window.

---

## What this retro does NOT cover

- **`AgentMuxFloatingPane` window class without `dir_hash`** (I5 violation) — tracked as a separate item; dormant when only one instance runs.
- **`GetMessageW(nullptr)` HWND guard** — added in this fix; root cause documented above.
- **Backend layout write atomicity outside saga boundary** — pre-existing gap F1.A; out of scope for this PR.
- **macOS/Linux redock cursor coordinates** — `BeginWindowDrag` F3 spike still pending; `window_drag_ended { moved: false }` now resets `dragging` but does not yet surface cursor coordinates for `tryRedockAtCursor` on non-Windows.
