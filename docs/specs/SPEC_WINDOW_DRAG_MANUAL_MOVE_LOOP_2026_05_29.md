---
title: Window drag — host-side manual native move loop (Windows)
status: Approved — implementing
date: 2026-05-29
author: AgentX
front: window-drag (UX-latency umbrella #1161)
supersedes_approach: "Option B (raw WM_NCLBUTTONDOWN) in SPEC_WINDOW_DRAG_NATIVE_MOVE_LOOP_2026_05_29.md — proven dead, see §1"
related:
  - "docs/specs/SPEC_WINDOW_DRAG_NATIVE_MOVE_LOOP_2026_05_29.md (problem statement + Option A/B)"
  - "docs/analysis/ANALYSIS_UX_LATENCY_THREE_FRONTS_2026_05_29.md"
  - "docs/specs/SPEC_WINDOW_DRAG_DPI_FIX_2026-05-13.md (the DPI hack this removes)"
---

# Window drag — host-side manual native move loop (Windows)

## 1. Spike result — why the OS move loop can't be reused

The previous spike wired the frontend to fire one `start_window_drag` on the 4px
threshold and had the host run `ReleaseCapture()` + `SendMessageW(WM_NCLBUTTONDOWN,
HTCAPTION)`. First attempt failed because it ran on the tokio IPC worker, where
`ReleaseCapture()` is a no-op (it's thread-specific). After marshaling it onto the
**CEF UI thread**, the instrumented log was conclusive:

```
begin-move on UI thread  hwnd=0x14b00fec  capture_before=0x14b00fec  release_ok=true
move loop returned       hwnd=0x14b00fec        ← 0.4 ms later
```

`release_ok=true` (threading is correct now), but `SendMessageW(WM_NCLBUTTONDOWN)`
**returns in 0.4 ms instead of blocking for the drag** — i.e. the OS modal move loop
**never engages**. Chromium's custom window frame (`HWNDMessageHandler`) intercepts
non-client messages and only runs an OS window-drag for `-webkit-app-region:drag`
regions — which AgentMux can't use because they suppress all renderer events (and
kill the title-bar context menu). **The OS will not run the move loop for a CEF
window, so we run it ourselves**, natively, on the UI thread — no per-move IPC.

This is the deliberate realization of the original "freeze the content, do the move,
update on release" idea, placed in the native layer instead of fighting it in JS.

## 2. Goal & non-goals

**Goal:** title-bar drag tracks the cursor at input rate (VS Code-smooth), zero
per-move IPC, no `devicePixelRatio` hand-correction, right-click + double-click
preserved.

**Non-goals (v1):** Aero Snap / snap-assist (see §6 tradeoff), live content updates
*during* the drag (content shows its last frame and moves with the window — by
design), tab tear-off (separate gesture, unchanged).

## 3. Architecture

Frontend is **unchanged** from the current native path (`useWindowDrag.win32.ts`):
arm on mousedown over a `data-drag-region`, and on the 4px threshold fire **one**
fire-and-forget `start_window_drag` IPC. Everything else is host-side.

Host: `start_window_drag` (motion.rs) resolves the top-level HWND label-aware
(thread-safe) and posts `Win32BeginMoveTask` to the **CEF UI thread** (already wired
as `post_win32_begin_move`). The task runs a **manual move loop** on the UI thread.

```
renderer mousedown→4px ─▶ start_window_drag IPC ─▶ motion.rs resolve HWND
                                                  └▶ post_task(UI, Win32BeginMoveTask)
                                                        └▶ manual move loop (this spec)
```

## 4. The manual move loop (UI thread)

Pseudocode (Rust / windows-sys; full impl in `ui_tasks.rs::Win32BeginMoveTask`):

```
fn execute():
  h = self.hwnd as HWND
  // 0. Bail if the press already ended (fast click / late task).
  if !(GetAsyncKeyState(VK_LBUTTON) & 0x8000): return

  // 1. Take capture so all mouse input routes to THIS (UI) thread's queue.
  ReleaseCapture(); SetCapture(h)

  // 2. Anchor in PHYSICAL screen px — no DPI math needed.
  GetCursorPos(&anchor); GetWindowRect(h, &r); (x0,y0) = (r.left, r.top)

  // 3. Pump our own modal loop.
  loop:
    if !(GetAsyncKeyState(VK_LBUTTON) & 0x8000): break        // safety: button up
    if GetMessageW(&msg, NULL, 0,0) <= 0: { repost WM_QUIT; break }
    match msg.message:
      WM_MOUSEMOVE:
        GetCursorPos(&cur)
        SetWindowPos(h, NULL, x0+(cur.x-anchor.x), y0+(cur.y-anchor.y),
                     0,0, SWP_NOSIZE|SWP_NOZORDER|SWP_NOACTIVATE)
      WM_LBUTTONUP:
        DispatchMessageW(&msg)   // let Chromium see the up (input-state balance, §5)
        break
      WM_KEYDOWN if wParam==VK_ESCAPE:
        SetWindowPos(h, NULL, x0,y0, 0,0, SWP_NOSIZE|SWP_NOZORDER|SWP_NOACTIVATE) // cancel → restore
        break
      _: TranslateMessage(&msg); DispatchMessageW(&msg)        // keep app alive (paint/DPI/etc)

  // 4. Always release.
  ReleaseCapture()
```

**Coordinate model:** `GetCursorPos`, `GetWindowRect`, `SetWindowPos` are all
physical px → the `* devicePixelRatio` correction (and `SPEC_WINDOW_DRAG_DPI_FIX`)
is **deleted**. A mid-drag monitor crossing with a different scale fires
`WM_DPICHANGED`, which is dispatched in the `_` arm (Chromium resizes); the anchor
delta stays valid because we re-read `GetCursorPos` each move and offset from the
fixed `(x0,y0)` origin (which itself moved only via our `SetWindowPos`). If a DPI
resize is observed to desync the origin in testing, re-read `GetWindowRect` on
`WM_DPICHANGED` (listed as a follow-up, not v1-blocking).

## 5. Sharp edges (designed-for)

1. **Renderer input-state consistency (highest risk).** The renderer received the
   original `mousedown` (web content). While we hold capture and *consume*
   `WM_MOUSEMOVE`/`WM_LBUTTONUP`, the renderer does **not** see them, so without care
   Chromium can be left believing a button is still down (stuck selection / odd next
   click). **Mitigation (v1):** `DispatchMessageW` the terminating `WM_LBUTTONUP` (do
   not just `break` on it) so Chromium completes the down→up pair. **Must
   runtime-verify**: after a drag, click a button / select text in the window behaves
   normally. If this proves insufficient, fall back to the **no-capture variant**
   (§7) which never steals the renderer's input.

2. **Nested modal loop.** Running `GetMessageW` inside a CEF UI-thread task is a
   nested message loop; CEF's own loop is suspended until we return. Standard for
   modal drags, but: (a) CEF `post_task` work queues until release (fine for a short
   drag), (b) never hold an `AppState`/parking_lot lock across the loop (we don't),
   (c) handle `GetMessageW` returning `0` (WM_QUIT) by reposting `PostQuitMessage` and
   bailing so app-quit isn't swallowed.

3. **Button released off-window / capture stolen.** `WM_CAPTURECHANGED` is *sent*
   (not posted), so it won't surface in `GetMessageW`; the per-iteration
   `GetAsyncKeyState(VK_LBUTTON)` check is the safety net that ends the loop if the
   up is missed.

4. **Re-entrancy / double start.** The frontend sets `dragInitiated=true` and stops
   tracking after firing once, so only one task starts per press. The `GetAsyncKeyState`
   bail (step 0) covers a late task whose press already ended.

5. **Floating panes.** HWND is resolved label-aware (`resolve_window_hwnd(label)`),
   so a floating pane drags itself, not main.

## 6. Tradeoffs vs Option A (CEF `BeginWindowDrag`)

| | B′ manual loop (this spec) | A: CEF BeginWindowDrag |
|---|---|---|
| CEF patch / libcef rebuild | **None** | Required (Win32 impl + `patched-libcef`) |
| Aero Snap / snap-assist | **Lost** (v1) — can add manual edge-snap later | Free (OS owns the move) |
| Renderer input consistency | Must manage (§5.1) | Clean (goes through Chromium input) |
| Content during drag | Frozen last frame moves w/ window | Same |
| Effort | **Medium**, in-tree | Large (build infra) |

B′ is the pragmatic ship-now path. If §5.1 can't be made clean, A is the fallback.

## 7. Alternative kept on file — no-capture poll

If capture+consume causes renderer-input issues: **don't `SetCapture`**; instead poll
`GetCursorPos` + `GetAsyncKeyState(VK_LBUTTON)` each frame, `SetWindowPos` the delta,
and pump pending messages via `PeekMessageW` + a `MsgWaitForMultipleObjectsEx(~8ms)`
wait. The renderer keeps its input (gets its own `mouseup` → consistent state); cost
is a busier loop. Slightly less smooth, strictly safer for input state.

## 8. Flag & rollout

- Native is default; legacy JS path stays behind `localStorage['agentmux.win32NativeDrag']='0'`
  (already implemented) for one release as a fallback.
- Phased: land behind the flag → run the bench + manual matrix → flip default → delete
  the JS path next release.

## 9. Measurement (gate for merge)

Extend the **#1176 input-latency bench** with a drag harness measuring **cursor→window
lag** (sample window rect via the WRR `LOCATIONCHANGE` stream vs synthetic cursor):
- **Success:** lag ≈ one frame and **flat under injected IPC jitter** (the move left
  the IPC path entirely — there is no per-move IPC anymore).
- `set_window_position` call-count during a drag = **0**; exactly one `start_window_drag`
  per drag.
- Re-run @ 100/125/150% scale — the 125% lag must be **gone** (DPI math removed).
- Existing `[start_window_drag] manual move loop …` host logs confirm engagement.

## 10. Acceptance criteria

- [ ] Drag tracks the cursor smoothly (no jitter), lag flat & jitter-insensitive.
- [ ] No regression @ 125/150% scale (expect improvement).
- [ ] **After a drag, the window's UI is fully interactive** (no stuck button/selection
      state) — the §5.1 verification.
- [ ] Right-click title-bar menu + double-click-maximize still work.
- [ ] Esc mid-drag cancels and restores the start position.
- [ ] Floating pane drags itself, not main.
- [ ] JS path still reachable via the localStorage flag.

## 11. Out of scope / follow-ups

- Aero Snap (manual edge detection + snap layouts).
- `WM_DPICHANGED` origin re-read if cross-monitor desync is observed.
- Porting to a single cross-platform path if Option A's `BeginWindowDrag` later lands
  on Windows.
