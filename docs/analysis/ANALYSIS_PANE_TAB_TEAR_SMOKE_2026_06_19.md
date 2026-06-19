# Pane / Tab Tear Smoke-Test Findings — 2026-06-19

Live diagnostic session: `task dev` on macOS 26 Tahoe (Darwin 25.5.0, arm64).
Monitor filter: errors, warnings, perf, drag/tear/drop signals.

---

## Summary

Three distinct issues observed during smoke testing:

| # | Issue | Severity | Files |
|---|-------|----------|-------|
| 1 | Pool "exhaustion" on macOS — by design, Phase 7 gap | Info | `agentmux-cef/src/commands/window_pool.rs:477` |
| 2 | Ghost indicator missing when dwell arm fires at the IPC boundary | Bug | `frontend/app/workspace/floating-pane-workspace.tsx:777-815` |
| 3 | Wrong-tab block delete on floating pane close (BUG-TRACE loop) | Bug | `frontend/app/tab/tabcontent.tsx:61-71` |

---

## Issue 1 — Pool Exhaustion Warning is Misleading (macOS by Design)

### Observed

```
WARN dnd:tearoff:pool: [pool] pool exhausted on tear-off — frontend will cold-path workspace_id=…
WARN dnd:tearoff:pool: [fe] pool promote failed, cold-pathing {"error":"Error: pool_exhausted"}
```

Every tab tear falls through to `open_window_at_position` (cold path), incurring the full
window-init cost (~350ms of sequential IPC calls: `get_platform`, `get_config_dir`,
`get_user_name`, etc., each taking 20-55ms on macOS in dev).

### Root Cause

`init_pool` is gated behind `#[cfg(target_os = "windows")]`:

```rust
// window_pool.rs:477-496
pub fn init_pool(state: &Arc<AppState>) {
    #[cfg(not(target_os = "windows"))]
    {
        let _ = state;
        return;          // ← macOS/Linux: no-op, pool never populated
    }
    // Windows only …
}
```

`promote_pool_window` on non-Windows similarly:

```rust
// window_pool.rs:907-920
#[cfg(not(target_os = "windows"))]
pub fn promote_pool_window(…) -> Option<String> {
    None  // Phase 7 will add equivalents
}
```

### Status

By design. Phase 7 is supposed to port the pool to macOS/Linux. The WARN log is
misleading because it implies the pool *ran out* when it was never populated. The log
target `dnd:tearoff:pool` should distinguish "pool not implemented on this platform"
from "pool was populated but exhausted."

### Cold-Path Latency Observed

Each cold-path tear spawns a full `initTauriNewWindow` sequence:
- ~12 IPC calls × 20-55ms each ≈ 300-400ms of startup IPC on the tear critical path
- `object.GetObject` for the torn tab: 53-156ms on first fetch in the new window
- Two consecutive long-tasks (50ms + 80ms) in the new window on init
- `initTauriNewWindow` fires **twice** for the same `workspaceId` on each cold tear
  (double-init bug — needs separate investigation)

Also flagged during compilation: `start_tear_off_tracking` in
`agentmux-cef/src/commands/tear_off_hook.rs:236` is never called. This function
appears to be the hook for the warm-path tear tracking that only works with a live pool.

---

## Issue 2 — Ghost Indicator Race: Dwell Arms Before IPC Returns

### Symptom (user-reported)

> "There are times I try to redock, no ghost appears, I let go thinking it won't
> redock because there was no landing ghost, but it docks anyway."

### Flow (macOS JS-driven drag path)

On macOS, floating pane drag is JS-driven (`jsDrivenDrag = true`, `floating-pane-workspace.tsx:163`)
because `BeginWindowDrag` requires a patched libcef absent in dev builds (PR #1308).

The `mousemove` handler (`floating-pane-workspace.tsx:732`) drives both position
updates and the redock hover:

```ts
// Line 776-780
if (dwellSlowSince === null) dwellSlowSince = now;
if (now - dwellSlowSince < REDOCK_DWELL_MS) return;   // ← gate: 180ms slow movement
if (now - lastHoverAt < HOVER_THROTTLE_MS) return;     // ← 50ms throttle
lastHoverAt = now;
// → first call to update_floating_redock_hover fires here (T = REDOCK_DWELL_MS)
```

The redock arm condition in `onMouseUp` (`floating-pane-workspace.tsx:522-527`):

```ts
const armed = hoverArmed ||
    (dwellSlowSince !== null && nowMs - dwellSlowSince >= REDOCK_DWELL_MS) ||   // ← arm #2
    (dwellCurrentConfirmedAt !== null && nowMs - dwellCurrentConfirmedAt >= REDOCK_DWELL_MS);
```

### Race Window

`update_floating_redock_hover` is only sent at T = `REDOCK_DWELL_MS`. The IPC
round-trip is 20-50ms (observed). `indicatorShowing` (and therefore the ghost in the
target window) is only set when the IPC `.then()` fires with a non-null target.

If the user releases the mouse at T = `REDOCK_DWELL_MS` (exactly when arm condition #2
fires via `dwellSlowSince`), the IPC is still in flight:
- `armed = true` → `tryRedockAtCursor` fires → **block docks**
- IPC not yet returned → `indicatorShowing = false` → **ghost never shown**

This is a 20-50ms race window that users hit regularly when they release the mouse
naturally as soon as the window "feels" like it's over the target.

### Fix Direction

Send `update_floating_redock_hover` slightly **before** the arm threshold so the IPC
round-trip completes by the time `onMouseUp` evaluates arm conditions. For example,
start sending at `REDOCK_DWELL_MS - HOVER_THROTTLE_MS` (i.e. 130ms slow dwell instead
of 180ms), so the first IPC fires with ~50ms runway before the arm threshold elapses.

```ts
// proposed: fire IPC at 130ms, arm redock at 180ms
const HOVER_LEAD_MS = HOVER_THROTTLE_MS;   // 50ms lead
if (now - dwellSlowSince < REDOCK_DWELL_MS - HOVER_LEAD_MS) return;
if (now - lastHoverAt < HOVER_THROTTLE_MS) return;
```

Alternatively, show an optimistic ghost immediately when `dwellSlowSince >= REDOCK_DWELL_MS`
(before IPC returns), with a fallback clear if the IPC returns null.

---

## Issue 3 — Wrong-Tab Block Delete on Floating Pane Close (BUG-TRACE)

### Observed (repeating on every floating pane close)

```
[BUG-TRACE] onNodeDelete ERROR: Error: call object.DeleteBlock error:
  DeleteBlock: block bc98efcd-367a-437a-8403-3abbd698ff59
  is in tab c51f12c9-cced-4049-b4b8-6d4d6da71363,
  not 93fc9be0-c93f-47f5-8a60-a0c59d7de5c3
```

The same block ID (`bc98efcd`) consistently fails across multiple closes, always
because it's claimed by the source tab (`c51f12c9`) rather than the floating window's
tab (which changes each time: `93fc9be0`, `97b33da6`, `a83370ef`).

### Root Cause

`onNodeDelete` in `frontend/app/tab/tabcontent.tsx:61-71`:

```ts
async function onNodeDelete(data: TabLayoutData) {
    const result = await services.ObjectService.DeleteBlock(data.blockId);
    …
}
```

`tileLayoutContents` (same file, line 52-80) is created with `tabId: props.tabId`
(the floating window's own tab). The `DeleteBlock` service validates that the block
belongs to the calling context's tab — but when a pane is torn to a floating window,
the block's ownership in the server's store remains with the **source tab**, not the
floating window's tab.

When the floating window is closed, `onNodeDelete` fires in the floating window's
context. `DeleteBlock` is called with the correct `blockId` but the server rejects it
because the block isn't owned by the floating tab.

### Impact

- The `DeleteBlock` call throws on every floating pane close
- The error is caught and logged but the block is not cleaned up
- This leaves the block orphaned in the source tab's layout, which is why panes can
  sometimes "reappear" after a floating close or leave stale layout state

### Fix Direction

Two options:

**A. Transfer block ownership on float** — when a pane tears to a floating window,
move the block to the floating tab's ownership in the server store. On redock/close,
move it back. This matches the mental model but requires server-side saga changes.

**B. Server-side: skip tab validation in `DeleteBlock`** — or add a
`ForceDeleteBlock(blockId)` variant that doesn't check tab ownership. Simpler but
less safe (any window could delete any block).

**C. Frontend: look up the block's actual tab before deleting** — call a
`GetBlockTab(blockId)` IPC before `DeleteBlock` and pass the correct tab. Avoids
server changes but adds an IPC round-trip.

Option A is the correct fix long-term. Option C is the easiest short-term patch
without touching the server store.

---

## Other Signals Logged (Not Root Causes)

| Signal | Location | Notes |
|--------|----------|-------|
| `computations/cleanups created outside createRoot` | SolidJS frontend | Reactive leaks on every window creation; potential for stale signal owners during drag |
| `ResizeObserver loop completed with undelivered notifications` | Tab bar | Fires on each new tab; layout thrash during tab init |
| `SetApplicationIsDaemon: Error -50 paramErr` | macOS sandbox | Fires at drag start; `NSOSStatusErrorDomain -50` on `isHandlingSendEvent` stub injection |
| `split_horizontal`, `split_vertical`, `split_impl` never used | `agentmux-cef/src/…/layout` (compile warn) | Dead layout split code; may be precursors to pane-split tear |
| `start_tear_off_tracking` never called | `tear_off_hook.rs:236` | Warm-path tear hook, currently unreachable |
| Startup IPC latency 49-66ms per call | macOS dev | 12 sequential calls × ~50ms = ~600ms cold-path window init total |
| `main_window_focus` IPC 58ms | macOS dev | Focus round-trip on every new window |

---

## Session Details

- Build: `agentmux-srv 0.46.4-darwin.arm64` + Vite 6.4.1 on port 5360
- Platform: macOS 26 Tahoe (Darwin 25.5.0)
- Tabs opened during test: 3
- Tab tears performed: 1 tab tear → cold path; 2 pane tears → floating windows
- Floating pane closes: 2 (both hit BUG-TRACE block delete)
- Window drags: several (3 long-tasks per drag: 51ms → 162ms → 138ms pattern)
