# Tab tear-off — native drag loop (Chrome's Win32/X11 model)

**Created:** 2026-05-07
**Owner:** AgentA
**Status:** SPEC ONLY — never built, never tried in this codebase
**Predecessors:** PR #727 (size match), PR #730 (position match + threshold), [`RESEARCH_TAB_TEAROFF_CROSS_PLATFORM_2026-05-07.md`](./RESEARCH_TAB_TEAROFF_CROSS_PLATFORM_2026-05-07.md)
**Effort estimate:** 2-3 days Win32 only, 5-8 days cross-platform (Win/macOS/X11/Wayland)

## 1. What "Chrome's actual Win32 implementation" means

When the research report says "Chrome runs a custom drag loop" on Win32/X11, here's what that concretely is in [`chromium/src/chrome/browser/ui/views/tabs/tab_drag_controller.cc`](https://github.com/chromium/chromium/blob/main/chrome/browser/ui/views/tabs/tab_drag_controller.cc):

1. **No HTML5 drag.** Tab-tear-off is detected at the C++ Views layer via `OnMousePressed` / `OnMouseDragged` — bypassing the HTML5 `dragstart` event entirely. There is no OLE drag, no `dataTransfer`, no `setDragImage`.
2. **Synchronous mouse capture.** On detection of a tear-off-bound drag (cursor leaves tab strip vertically), `gfx::Widget::SetCapture()` (Win32 `SetCapture()`) takes ownership of the cursor.
3. **Per-frame window-follow.** A C++ event handler (`MouseDragged` callback fires per OS mouse-move event, ~120Hz on modern systems) calls `gfx::Widget::SetBounds()` (Win32 `SetWindowPos()`) on the new tab's freshly-spawned window so the window literally chases the cursor at full opacity.
4. **No timer.** It's event-driven on `WM_MOUSEMOVE`. No polling, no debouncing.
5. **OS button-state-aware.** The drag loop ends when `WM_LBUTTONUP` fires.

The DEV.to "Implementing Chrome-Style Tab Tear-off in WinUI 3" article reproduces this in WinUI 3 with `DispatcherTimer + GetAsyncKeyState` because WinUI 3's IXP layer filters NC messages — they're working around a framework limitation that Chrome doesn't have because Chrome owns the message pump.

**Cross-platform note:** Chrome's Win32 path doesn't work on Wayland (no global cursor coords, no direct window positioning). Chromium falls back to bitmap-snapshot HTML5 DnD on Wayland — that's what we tried and rejected as flaky.

## 2. Has this been tried in AgentMux?

**No, not this approach.** What HAS been tried:

| Approach | Status | Where |
|---|---|---|
| HTML5 drag + SC_MOVE handshake on drop | shipped | `tearOffSCMoveHandshake` in `agentmux-cef/src/commands/drag.rs` |
| HTML5 drag + tear-off mid-drag at threshold | shipped via PR #730 | `tabbar.tsx::performTabTearOff` |
| HTML5 drag + bitmap snapshot drag image (modern-screenshot) | tried locally, reverted | this branch (snapshot-drag-image) — too async, too flaky |
| **Raw mousedown bypass + SetCapture + per-frame SetWindowPos** | **never tried** | n/a |

The legacy `tear_off_sc_move_handshake` in the Rust host was *intended* to give the Chrome-style live-window experience. The plan was: on threshold cross, post `WM_SYSCOMMAND/SC_MOVE` to the new window so Windows enters its built-in modal move-loop and follows the cursor. Smoke testing on v0.33.703 confirms **this does not work** because:

- pragmatic-dnd has already started an HTML5/OLE drag at this point
- OLE drag holds mouse capture on the source webview's thread
- `SC_MOVE` on the new window's thread can't take the modal move-loop while OLE owns capture
- Net result: `SC_MOVE` queues idle; new window stays at its `SetWindowPos` placement; on `mouseup` the OLE drag ends → SC_MOVE finally runs but mouse button is already up → modal drag exits immediately

## 3. The proposed path (this spec)

### 3.1 Frontend changes

**Stop using HTML5 drag for the tear-off gesture.** Two states:

- **In-bar reorder** — keep using HTML5 drag (pragmatic-dnd) as today. Works fine for cursor-stays-in-tab-bar.
- **Tear-off** — bypass HTML5 entirely. Detect via raw `mousedown` + `pointermove` on the tab.

State machine on each tab:

```
idle
  → mousedown → tracking (record initial cursor + tab geometry)
  → pointermove (cursor still in bar) → BAIL into pragmatic-dnd HTML5 drag (reorder mode)
  → pointermove (cursor leaves bar by ≥ TEAR_PAST_PX) → fire tear-off
  → mouseup before either → click (select tab)
```

Once "fire tear-off" triggers:
1. Frontend calls `requestTearOff` (existing flow) → host spawns new window via warm pool (existing) → window appears at anchor position (existing PR #730 fix).
2. Frontend calls a new `engageNativeWindowDrag` host RPC, passing the new window's label + current cursor position.
3. Frontend continues to receive `pointermove` events (because the source webview never lost mouse capture — there was no HTML5 drag).
4. Each `pointermove` → throttled IPC (one in flight at a time, coalesce skipped frames) → host calls `SetWindowPos` on the new window's HWND to follow cursor.
5. `mouseup` → final IPC → host releases tracking, leaves new window at final position.

### 3.2 Host changes

**New RPC `engageNativeWindowDrag(label, cursorX, cursorY)`** — installs the new window's HWND as the "currently following" target. Rust holds an `Arc<Mutex<Option<DragTarget>>>` with the HWND.

**New RPC `updateNativeWindowDrag(cursorX, cursorY)`** — `SetWindowPos(target_hwnd, HWND_TOP, cursorX - inset_x, cursorY - inset_y, 0, 0, SWP_NOSIZE)`. Single-mutex-lock + single Win32 call. <1ms.

**New RPC `endNativeWindowDrag()`** — clears the target.

**Why a tracked-state RPC instead of including the HWND in every message?** Avoids re-resolving the label→HWND every frame; Mutex lookup is cheaper than HashMap.get + browser_handle.host.

### 3.3 Throttling for the per-frame IPC

`pointermove` fires every native mouse event (~120Hz on a typical mouse, up to 1000Hz on gaming mice). 120 IPCs per second is too much.

Strategy: one-in-flight + coalesce.
```ts
let inFlight = false;
let pending: { x: number; y: number } | null = null;

function onPointerMove(e: PointerEvent) {
    if (inFlight) {
        pending = { x: e.screenX, y: e.screenY };
        return;
    }
    inFlight = true;
    invokeCommand("update_native_window_drag", { cursorX: e.screenX, cursorY: e.screenY })
        .finally(() => {
            inFlight = false;
            if (pending) {
                const { x, y } = pending;
                pending = null;
                onPointerMove({ screenX: x, screenY: y } as any);
            }
        });
}
```

This pegs at the IPC RTT (~1-3ms typical) so we hit ~300-1000Hz worst case. If RTT spikes, we drop intermediate frames and use the latest. No queue buildup.

### 3.4 Cancel-back / drop-on-source

Existing PR #730 mid-drag-cross-back-into-source-bar logic still works. Under this spec, if `pointermove` returns to the source bar, we:
1. Call `endNativeWindowDrag()`
2. Call existing cancel-back path (close new window, reinsert tab)

### 3.5 Cross-platform

| Platform | Approach |
|---|---|
| **Win32** | this spec — `SetCapture`/`SetWindowPos`. Verified to work in Chromium. |
| **macOS** | `NSWindow performWindowDragWithEvent:` with the synthesized button-down NSEvent. Cocoa's native modal drag handles the follow-cursor — different API, similar effect. |
| **X11 / linux** | `_NET_WM_MOVERESIZE` window manager hint. Most WMs respect it. |
| **Wayland** | Path doesn't exist — Wayland forbids both global cursor coords AND direct window positioning. Fall back to PR #730's behavior + bitmap-snapshot drag image (the spec we tried and rejected — but on Wayland the bitmap-snapshot IS the production-quality approach, so its flakiness on Win32 doesn't apply). |

Each platform branch is a separate ~1-2 day implementation.

## 4. Risks

1. **Source webview's mouse capture during pointermove.** When the user clicks-and-drags, does the OS keep delivering pointermove events to the source webview after the cursor leaves the source window's bounds? On Win32, only with explicit `SetCapture()`. Frontend can't call `SetCapture` directly — needs a host RPC `setMouseCaptureOnSource`. Adds another piece to the choreography.
2. **CEF browser-pane interactions.** Source webview is a CEF Browser. Browser may swallow `pointermove` for its own purposes during drag. Need to verify.
3. **Multi-monitor + DPI.** PR #730 already grappled with this for the SetWindowPos call; same considerations apply.
4. **Race between window-spawn and first pointermove.** New window must be HWND-registered before `engageNativeWindowDrag` succeeds. Existing `wait_for_browser_hwnd` polls — would need to do that.
5. **Cancellation paths.** ESC, focus-loss, app-quit-mid-drag must all release the captured cursor cleanly or the user is stuck unable to click anything.

## 5. Open questions for spec review

1. Is `pointermove` reliable across the source-window boundary in CEF v146 without explicit `SetCapture`? *Need to test.*
2. Does Chrome's Wayland-fallback bitmap-snapshot path work in CEF's Wayland mode? Tested above on Win32 (reverted); Wayland may have better behavior because the OS expects HTML5 drag everywhere.
3. Is the per-frame IPC throughput acceptable? Spike with ~5 minutes of code (`setInterval` calling `SetWindowPos` from frontend, measure jank).
4. What's the user's appetite for partial cross-platform? Win32-only is shippable but excludes macOS users. Win32+macOS reasonable scope; deferring linux/wayland is also reasonable.

## 6. Recommended path forward

**Two-phase implementation:**

- **Phase 1 (Win32 only, ~2-3 days)** — proof-of-concept on Windows. Establishes the IPC contract + per-frame loop shape. Smoke + ship to Windows-only users.
- **Phase 2 (macOS + Linux, ~3-5 days)** — port to NSWindow/X11. Wayland users get PR #730's behavior unchanged.

**Before Phase 1: 1-day spike** to answer the open questions, especially #1 (pointermove across window boundary). If that doesn't work, the whole spec needs rework — possibly Win32 hooks (WH_MOUSE_LL like the existing `tear_off_hook.rs`) instead of pointermove.

## 7. What we should NOT do

- Continue iterating on bitmap-snapshot DOM-to-image. It's fundamentally async, fundamentally racy with the OS's synchronous drag-image capture. Reverted from this branch. Don't re-try without a different mechanism (host-side capture, OS API).
- Add timers / debounce hacks. The user is right: timers make tab interactions sluggish. Real-time drag must be event-driven.
- Try to make HTML5 drag + SC_MOVE work. Five smoke sessions confirm it doesn't. The OLE-capture-blocks-SC_MOVE path is a dead end.
