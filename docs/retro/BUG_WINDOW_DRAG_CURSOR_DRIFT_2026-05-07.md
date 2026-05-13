# Bug Report: Window Drag Cursor Drift

**Date:** 2026-05-07  
**Symptom:** When dragging the title bar to move the window, the cursor's position on the bar drifts — it does not stay at the point where the user originally clicked.  
**Severity:** P2 — noticeable UX regression, not a crash  
**Area:** `frontend/app/hook/useWindowDrag.win32.ts`, `agentmux-cef/src/commands/window.rs`

---

## Observed Behaviour

User clicks 100 px from the left edge of the title bar and drags right. As the drag continues the cursor gradually slides toward the right edge of the bar; by the time the window has moved ~200 px the cursor is visibly offset from the original click point.

---

## Code Path

### 1. Frontend — `useWindowDrag.win32.ts`

Because CEF does not expose `WM_NCLBUTTONDOWN`-style OS-native drag (comment at line 7), AgentMux implements dragging entirely in JS: track mouse deltas and forward them to the Rust host via IPC.

```
mousedown  → capture click position in startScreenX/Y (lines 37-38)
           → fire get_window_position IPC (line 42) — NOT awaited, result discarded
mousemove  → dx = e.screenX − startScreenX           (line 49)
           → dy = e.screenY − startScreenY           (line 50)
           → startScreenX = e.screenX                (line 52)  ← rolling baseline
           → startScreenY = e.screenY                (line 53)  ← rolling baseline
           → invokeCommand("move_window_by", {dx,dy}) (line 54) ← fire-and-forget
mouseup    → dragging = false
```

### 2. Rust host — `agentmux-cef/src/commands/window.rs`

`move_window_by` (lines 179–209):

```rust
GetWindowRect(hwnd, &mut rect);          // read current position
SetWindowPos(hwnd, ..., rect.left + dx, rect.top + dy, ...);
```

No `set_window_position` (absolute) command exists. The only positioning API is relative (`move_window_by`).

---

## Root Cause

The bug is a **stale-read race** between concurrent in-flight IPC calls, amplified by the fire-and-forget dispatch pattern.

### Why the delta approach is fragile under load

At normal mouse speeds mousemove fires 60–200+ events per second. Each event:
1. Computes a delta from the rolling baseline (`startScreenX`)
2. **Immediately advances** `startScreenX` to the current position
3. Sends `move_window_by({dx, dy})` as a fire-and-forget IPC — no await, no back-pressure

This means at any moment there are N in-flight IPC calls queued in the CEF IPC channel, each carrying a partial delta.

### The stale-rect scenario

`move_window_by` calls `GetWindowRect` then `SetWindowPos` in the Rust handler. In CEF the IPC handler runs on a non-UI (IO/render) thread. `SetWindowPos` called from a non-owning thread is relayed to the UI thread via the Windows message queue — it does **not** apply synchronously to the internal `RECT` that `GetWindowRect` reads.

Timeline with 4 rapid mousemoves (+10 px each, target: +40 px total):

| IPC call | GetWindowRect returns | SetWindowPos target | Window actually at |
|----------|-----------------------|---------------------|--------------------|
| IPC 1    | left = 100 (initial)  | 110                 | 100 (not yet applied) |
| IPC 2    | left = 100 (stale!)   | 110                 | 100 (still stale)  |
| IPC 3    | left = 100 (stale!)   | 110                 | 100 (still stale)  |
| IPC 4    | left = 100 (stale!)   | 110                 | 100 (still stale)  |
| DWM flush | —                    | —                   | 110 (last wins)    |

All four calls see the same old rect. The window moves +10 instead of +40. The cursor has moved +40 on screen. The cursor is now 30 px further right on the title bar than when the drag started — exactly the reported drift.

The severity of the drift scales with:
- Mouse speed (more events per SetWindowPos cycle → larger stale-read window)
- IPC round-trip latency (slower machines / heavier load → more staleness)
- DWM composition interval (higher refresh rate = faster flush = less visible; lower rate = worse)

### Why `get_window_position` on mousedown doesn't help

Line 42 calls `get_window_position()` but:
- The returned value is never stored (`invokeCommand<{x,y}>(...).catch(...)` — result is dropped)
- It is not awaited, so dragging begins before the initial position is known

This call was presumably added in anticipation of absolute positioning but was never wired up.

---

## Fix: Absolute Positioning

Switch from incremental deltas to absolute target coordinates. This eliminates the stale-read race entirely — each IPC call is independent and does not depend on the outcome of previous calls.

### Frontend changes (`useWindowDrag.win32.ts`)

```ts
let dragging = false;
let clickScreenX = 0;
let clickScreenY = 0;
let initWinX = 0;
let initWinY = 0;
let initFetched = false;

document.addEventListener("mousedown", async (e: MouseEvent) => {
    if (e.button !== 0 || !isInDragRegion(e.target as HTMLElement)) return;
    e.preventDefault();
    try {
        const pos = await invokeCommand<{ x: number; y: number }>("get_window_position");
        clickScreenX = e.screenX;
        clickScreenY = e.screenY;
        initWinX = pos.x;
        initWinY = pos.y;
        initFetched = true;
        dragging = true;
    } catch {
        // host unavailable — abort
    }
}, true);

document.addEventListener("mousemove", (e: MouseEvent) => {
    if (!dragging || !initFetched) return;
    const tx = initWinX + (e.screenX - clickScreenX);
    const ty = initWinY + (e.screenY - clickScreenY);
    invokeCommand("set_window_position", { x: tx, y: ty }).catch(() => {});
});
```

Key differences:
- `get_window_position` is now **awaited** — drag does not start until the initial position is known
- `clickScreenX/Y` is captured after the await (avoids the gap between click and IPC response)
- Deltas are computed from the **fixed** click origin, not a rolling baseline
- Each mousemove sends an absolute target, not a relative delta

### Rust host: add `set_window_position` (`commands/window.rs`)

```rust
pub fn set_window_position(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;

    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        use windows_sys::Win32::Foundation::RECT;
        let hwnd = find_own_top_level_window();
        if !hwnd.is_null() {
            let mut rect: RECT = std::mem::zeroed();
            GetWindowRect(hwnd, &mut rect);
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;
            SetWindowPos(hwnd, std::ptr::null_mut(), x, y, width, height, 0x0014);
            return Ok(serde_json::Value::Null);
        }
    }
    #[cfg(not(target_os = "windows"))]
    crate::ui_tasks::post_set_window_position(state, "main", x, y);
    let _ = state;
    Ok(serde_json::Value::Null)
}
```

Register in `ipc.rs` alongside the existing commands:

```rust
"set_window_position" => commands::window::set_window_position(state, args),
```

### Why absolute positioning is superior to fixing the delta approach

| Property | Delta + rolling baseline | Absolute (proposed) |
|----------|--------------------------|---------------------|
| Stale-read race | Yes — N in-flight IPCs all apply from same old rect | No — each call is self-contained |
| Cursor drift under fast mouse | Yes | No |
| Correctness under IPC backpressure | Degrades | Idempotent — last-write-wins is correct |
| Dropped events | Accumulate error | No effect on final position |
| `get_window_position` round-trip on mousedown | Already exists, just not used | Uses the existing call |

---

## Secondary Issue: `mousedown` Async Gap

With the await on `mousedown`, there is a brief window (~1–2 ms IPC round-trip) between the user's click and `dragging = true`. This is acceptable — the cursor is stationary at click time and the first `mousemove` events fire after the click handler completes. No perceptible delay.

If latency is a concern, the initial position can be pre-fetched on title-bar `mouseenter` and cached; the mousedown handler then uses the cached value synchronously.

---

## Files to Change

| File | Change |
|------|--------|
| `frontend/app/hook/useWindowDrag.win32.ts` | Switch to absolute positioning (await initial pos, fixed click origin) |
| `agentmux-cef/src/commands/window.rs` | Add `set_window_position(x, y)` |
| `agentmux-cef/src/ipc.rs` | Register `set_window_position` in the command router |
| `agentmux-cef/src/ui_tasks.rs` (if it exists) | Add `post_set_window_position` for non-Windows paths |

`move_window_by` and `get_window_position` can remain — they may be used elsewhere.

---

## Postscript — 2026-05-13: DPI regression

The fix above shipped (PR #734) and resolved the stale-rect race. **It also introduced a latent unit-mismatch bug** that this retro never caught because all testing was at 100% Windows scale (the Win10 default).

Symptom: cursor drifts off the click point during drag on **Windows 11**, which defaults to 125% scale on most laptops. Was masked on Windows 10 (100% default).

Root cause: `e.screenX` in CEF/Chromium with `use-zoom-for-dsf` (default on Windows since Chrome 54) is in **CSS pixels** (physical ÷ devicePixelRatio). `get_window_position` returns and `set_window_position` consumes **physical pixels** in the PMv2-aware host. The original fix added CSS-pixel deltas to a physical-pixel baseline:

```ts
// BUGGY (PR #734)
const tx = initWinX + (e.screenX - clickScreenX);
```

At Win11 125% (`dpr = 1.25`), the window moves 80% of the cursor distance — the same visible drift the original bug had, with a different mechanism.

Fixed in PR following `docs/specs/SPEC_WINDOW_DRAG_DPI_FIX_2026-05-13.md`. The spec also adds:

1. **DPR multiplier + `Math.round`** in the frontend hook (the core fix).
2. **Re-read `devicePixelRatio` per mousemove** so cross-monitor mid-drag at different scales picks up the new value.
3. **Recommended test matrix** for any future drag work: 100% / 125% / 150% / 175% / 200%, single + multi-monitor, homogeneous + differential scale. Adding to the smoke-test checklist as a standing item.

Lesson for future retros: **scale-sensitive Win32 work needs explicit DPI-matrix testing before merge.** A "tested at 100%" implicit assumption is invisible in the retro and bites at user install time.
