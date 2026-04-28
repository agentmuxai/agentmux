# Tab Tear-Off — Retrospective + Research (2026-04-27)

**Authors**: AgentA-asaf
**Status**: written after 9 review rounds on PR #567 (Phase 5 cancel-back, merged) and 2 follow-on attempts at fixing window-size preservation that did not land for the user.

---

## Why this document exists

User reported on the 0.33.433 portable build:

> "the tabs still dont drop as the same size. nor does the entire content drag like on chrome"

Then:

> "well need to take a big step back .. write a retro on everything you've tried and research online for best practices, write to new file, open in vscode"

This is a fair pause. We've been iterating in a loop — push, build, "still wrong," repeat — without revisiting whether the architecture itself is correct. The research turned out to reframe several decisions.

---

## What's actually in the build today (v0.33.433)

### Working (Win32 only)
- Tab tear-off threshold detection (Phase 1)
- SC_MOVE handshake — torn-off window follows cursor at full opacity, no ghost (Phase 2)
- WH_MOUSE_LL hook for cross-window merge detection (Phase 4)
- Cancel-back: ESC + drop-on-source-strip restores tab to original position, including pinned tabs (Phase 5)
- Pre-warmed window pool to eliminate first-paint flash (Phase 6)
- This session: pool windows hidden from taskbar via WS_EX_TOOLWINDOW (regression fix — Phase 6 had been leaking pre-warmed pool windows into the user's taskbar)

### NOT working (user-visible defects)
- **Defect 1** — torn-off window opens at default 1200×800 instead of matching source window dimensions
- **Defect 2** — torn-off window content is empty / shows AgentMux chrome only during the drag; Chrome shows the actual tab content alive during the drag

### Not implemented at all
- Phase 3 (size preservation per spec — TabSnapshot tab-element width)
- Phase 7 (cross-platform: macOS, Linux/X11, Linux/Wayland)

---

## What I tried for Defect 1 (window size) — and why each attempt failed

### Attempt 1 (v0.33.431): frontend `window.outerWidth × DPR`

Captured `window.outerWidth * devicePixelRatio` in `tabbar.tsx onDrag`, threaded through the frontend pipeline, applied via `SetWindowPos` in `promote_pool_window` / `open_window_at_position`.

**Result**: User reported the new window was still wrong size.

**Likely failure modes** (now informed by research):
- In CEF frameless mode, `window.outerWidth` may equal `window.innerWidth` (no native chrome to add), so we measured the content area, not the OS window.
- DPI conversion: `outerWidth` returns CSS pixels normalized to the source monitor's DPI. On multi-monitor setups with mixed DPI, `outerWidth × DPR` doesn't yield the right physical pixels for the destination monitor.

### Attempt 2 (v0.33.433): host-side `GetWindowRect(source HWND)`

Frontend passes `sourceWindowLabel`, host looks up source HWND via `state.browsers`, calls `GetWindowRect`, applies dimensions.

**Result**: User reported the new window was *still* wrong size.

**Likely failure modes** (now informed by research):
- `browser.host().window_handle()` returns the **CEF browser-widget HWND** (a child window inside the top-level frame), NOT the top-level. `GetWindowRect` on the widget gives the inner client area, not the outer window dimensions.
  - **Existing AgentMux code already knows this**: `tear_off_hook.rs:507` does `let root = unsafe { GetAncestor(hwnd, GA_ROOT) };` to walk up. My `source_window_dimensions` skipped that step.
- Per-monitor DPI: source HWND on monitor A at DPI 1.0; pool HWND on monitor B at DPI 2.0. `SetWindowPos` with monitor-A pixels on a monitor-B HWND scales wrong.
- Pool windows are pre-painted at `POOL_WIDTH = 1200`, `POOL_HEIGHT = 800`. `SetWindowPos` may apply but a subsequent CEF resize handler or DWM compositing might revert.

### What I should have done (and didn't)

- Add `tracing::info!` at every step of the dimension lookup so the user could send `muxlog host '\[dnd:tearoff\]'` output back, confirming exactly where the chain breaks. Instead I shipped two builds without diagnostic confirmation, then user said "still wrong" twice.
- Verify the assumption that `host.window_handle()` returns the top-level. The codebase already had counter-evidence (`tear_off_hook.rs` walks `GA_ROOT`).
- Ship a build to a test environment first, not to the user's primary instance.

---

## Research summary — Chrome / CEF / Electron tear-off (authoritative)

The following was researched specifically for this retro. Citations at the end.

### TL;DR

1. **Chrome on Windows DOES use `SendMessage(hwnd, WM_SYSCOMMAND, SC_MOVE | 0x0002, ...)`** — the same OS modal move-loop AgentMux is using. Our SC_MOVE choice is correct; our visual problems are NOT caused by SC_MOVE.
2. **The "content stays alive during drag" trick is not a bitmap, it's reparenting.** Chrome creates the destination `Browser` immediately when the tear threshold is crossed, moves the live `WebContents` into it, *then* hands control to the OS move-loop. The window the user drags around IS the real, final window, with its real, final content already mounted.
3. **The new window is sized from the source window's restored bounds**, with a 10px inset to prevent re-maximization. There is no fixed default; matching the source is the documented behavior.
4. **No mainstream CEF or Electron app implements true Chrome-style tear-off.** The Chrome behavior is built directly on Views/Aura internals that CEF and Electron deliberately do not expose. Apps that have "pop out" buttons (Slack, Notion, VSCode aux windows) explicitly do NOT support drag-tear; they require a button click and accept that the new window is a fresh frame.

### Chrome's TabDragController — actual mechanics

State machine: `kDraggingTabs` (within a tab strip), `kDraggingWindow` (a torn-off browser is following the cursor), and the transitions between them.

The pivotal method is **`DetachIntoNewBrowserAndRunMoveLoop`**. Sequence:

1. User crosses the vertical tear threshold above/below the tab strip.
2. A new `Browser` (and its `BrowserWindow`/widget) is constructed **synchronously**.
3. The `WebContents` (the actual live render process — "complete with animations" per the Mac design doc) is **reparented** from the source browser into the new browser.
4. State transitions to `kDraggingWindow`.
5. `Widget::RunMoveLoop` is invoked on the new browser's widget.
6. On Windows, `DesktopWindowTreeHostWin::RunMoveLoop` → `HWNDMessageHandler::RunMoveLoop` → ultimately `::SendMessage(hwnd(), WM_SYSCOMMAND, SC_MOVE | 0x0002, ::GetMessagePos())`.

This is straight from `hwnd_message_handler.cc`. So **Chrome on Windows uses exactly the same OS modal move-loop AgentMux is using**. The `SC_MOVE | 0x0002` form (low bit set) is the documented "as if the user clicked the title bar" variant.

While in `kDraggingWindow`, the controller watches the widget's bounds and uses a `WindowFinder` to detect when the cursor crosses another browser's tab strip — at which point it calls `EndMoveLoop()`, reparents the WebContents back, and returns to `kDraggingTabs`.

### Why the dragged content stays visible

It is **real-time render in the destination window with the WebContents already reparented**. Not a snapshot. Not a layered phantom. The window the user is dragging IS the final window, and it is rendering the live page (network, JS timers, video — all uninterrupted) the moment it appears.

Confirmation in the Mac design doc: *"Detaching the tab creates a new Browser ... containing the TabContents associated with the original tab. This allows it to maintain all its existing state and render processes (complete with animations!)."*

The **only** place Chrome uses a snapshot bitmap for tear-off is the **Wayland fallback** path, because Wayland clients can't reposition their own windows. There the drag icon shows a snapshot. On Windows / X11 / Mac it's reparenting.

### New window sizing — citation

Chrome derives the new window's bounds from the **source window's restored bounds**, NOT a fixed default. From the maximized-window CL: when the source is maximized, `TabDragController` reads the restore bounds; if those equal or exceed the work area it applies `kMaximizedWindowInset = 10` so the new window doesn't auto-re-maximize and become un-draggable. The relevant function in current code is `CalculateDraggedBrowserBounds` / `GetWindowCreatePoint` inside `tab_drag_controller.cc`.

So **AgentMux's 1200×800 default is the bug**; reading source `GetWindowRect` is the right idea, but you must:
- read the **top-level** window (not the CEF widget — `GetAncestor(GA_ROOT)`),
- read the **restored** bounds (not the maximized-bounds — `GetWindowPlacement`/`WPF_RESTORETOMAXIMIZED`),
- in **physical pixels** that match what `SetWindowPos` expects on the **destination** monitor's DPI (`GetDpiForWindow` + `AdjustWindowRectExForDpi`).

### CEF and Electron — the hard truth

**CEF**: cefclient ships no tab UI. Per the CEF forums, every CEF host that wants tabs builds its own native chrome and parents `CefBrowser`s into HWND tab containers. There is **no public CEF API for the WebContents reparenting that Chrome's TabDragController relies on** — `CefBrowser` is bound to its host window at creation. Tear-off in CEF is therefore *fundamentally* a different shape than Chrome's: you either close+reopen the browser in a new window (losing JS state), or you keep one browser alive and reparent the **HWND** that hosts it (possible, but you must own all the windowing code yourself).

**Electron**: `BrowserView` is deprecated for `WebContentsView`. The current `WebContentsView` API explicitly does NOT accept an existing WebContents at construction, which means **you cannot move a live WebContents from one BrowserWindow to another**. This is why VSCode's "drag tab to new window" is an open issue from 2018 (#53984) blocked on Electron-level changes. Slack, Notion, etc. expose "open in new window" only as a menu/button, never as a drag gesture, for the same reason.

The closest community implementation to Chrome's behavior on Windows is the WinUI 3 article on dev.to. Its key tricks worth stealing:

- **DWM cloaking** (`DwmSetWindowAttribute(DWMWA_CLOAK)`) for ~5 frames so the new window initializes invisibly and never shows a white flash.
- **Repeating the `SetWindowPos` for the first 3 frames** to defeat the framework's async `Activate()` resetting size.

---

## Architectural diagnosis

### Defect 1 — wrong window size: probable root cause

`browser.host().window_handle()` returns the CEF browser-widget HWND inside the top-level frame. Existing AgentMux code (e.g. `tear_off_hook.rs:507`) walks to the top-level via `GetAncestor(hwnd, GA_ROOT)` — but my `source_window_dimensions` does NOT, so it's measuring the widget, not the window.

Compounding: per-monitor DPI requires `GetDpiForWindow(source_hwnd)` to convert measurements correctly when the destination monitor has a different DPI.

**Fix shape** (next attempt — definitely add tracing this time):
1. `GetAncestor(host_handle, GA_ROOT)` to get the top-level
2. `GetWindowPlacement(top_level_hwnd)` and read `rcNormalPosition` (= restored bounds, not maximized)
3. Optionally `IsZoomed(top_level)` to detect maximized; apply Chrome's `kMaximizedWindowInset = 10` if so
4. `GetDpiForWindow(top_level_hwnd)` for source DPI; rescale to destination monitor DPI before applying
5. `tracing::info!` per step so we can confirm in `muxlog` without prompting the user

### Defect 2 — content not alive during drag: architectural mismatch

This is the real shock from the research. **Chrome works because it reparents the live `WebContents` from source browser into the new browser before the move loop starts.** The window the user drags IS the final window with its content already mounted.

**CEF does NOT expose this primitive.** `CefBrowser` is bound to its host HWND at creation. There is no public API to move a `CefBrowser` from window A to window B.

**This means our warm-pool architecture is wrong-shape for "content follows cursor"**. The pool window's content is `?pool=1` (a blank workspace) — it cannot have the source's tab content because that content lives in the source's `CefBrowser`, which can't be reparented across windows.

**Two viable paths forward** (mutually exclusive):

#### Path A — Bitmap snapshot fallback (Chrome's Wayland approach)

- At tear-time, capture source tab's client area as a bitmap (via CEF's `PrintToBitmap` / `Browser.GetImage` or GDI `BitBlt` on the source HWND)
- Display bitmap as a translucent overlay in a layered window during SC_MOVE
- On `WM_EXITSIZEMOVE`, the layered window dies and the destination window with real content takes over

Pros: incremental, keeps existing warm pool. Cons: the user sees a frozen bitmap (no live updates) during the drag. Explicitly Chrome's *fallback*, not its main path.

#### Path B — HWND reparenting (Chrome's main approach, adapted)

- Drop the warm pool entirely
- At tear-time, create a new top-level frame HWND, call `SetParent` to reparent the source's CefBrowser HWND into the new frame, then SC_MOVE on the new frame
- After mouseup, if no merge happened, the new frame becomes the destination; if merged, reparent back

Pros: single live render, no two-stage transition, true Chrome behavior. Cons:
- Removes the warm pool (wastes Phase 6 work)
- Requires CEF to tolerate HWND reparenting (untested; may break GPU compositing)
- Probably needs `SetWindowLongPtr(GWL_STYLE)` adjustments mid-drag
- Major refactor — multi-week, multi-PR

#### Path C — Ship-as-is, document limitation

- Accept "content blank during drag" as a known limitation
- Fix only window size (with `GetAncestor` + DPI fixes from §"Defect 1")
- Document in spec §0 that AgentMux's tear-off shows the destination window as a brief loading state during the drag, unlike Chrome
- Move to Phase 7 (cross-platform parity) and ship

---

## What "Chrome-faithful" actually requires (revised matrix)

| Behavior | Chrome | AgentMux today | Phase | Status |
|---|---|---|---|---|
| Threshold detection | yes | yes | 1 | ✅ |
| OS move loop | SC_MOVE | SC_MOVE | 2 | ✅ — confirmed correct |
| Content alive during drag | yes (WebContents reparenting) | no (pool is `?pool=1` blank) | — (was implicit in 0) | ❌ — needs Path A or B |
| Window size = source | yes | no (1200×800 default) | 3 | ❌ — fix shape known, just needs `GA_ROOT` + DPI |
| Cross-window merge | yes | yes | 4 | ✅ |
| ESC cancel | yes | yes | 5 | ✅ |
| Drop-on-source returns to original index | yes | yes | 5 | ✅ |
| Pinned-tab preservation across cancel-back | yes | yes (this session, PR #567 round-6) | 5 | ✅ |
| Cross-platform (macOS, Linux, Wayland) | yes | Win32 only | 7 | ❌ |
| First-paint flash | DWM cloaking + SC_MOVE | warm pool (works, but pool is wrong-shape for content) | 6 | ⚠️ — works for "no flash" but not for "content alive" |

---

## Recommended way forward — for user decision

**Decision 1: which approach for content-during-drag?**
- **(A)** Bitmap snapshot fallback — ~3 days, content visible but frozen
- **(B)** HWND reparenting — ~2 weeks, true Chrome behavior, drops Phase 6 pool, untested in CEF
- **(C)** Ship-as-is — document as known limitation; move to Phase 7

**Decision 2: window-size fix — independent of (1)**
- Rewrite `source_window_dimensions` with `GetAncestor + GetWindowPlacement + IsZoomed + GetDpiForWindow`
- Add per-step tracing so we don't ship blind again
- ~half-day

**Decision 3: Phase 6 (warm pool) — keep or drop?**
- If we go Path B (reparenting), warm pool becomes architecturally redundant — drop it
- If we go Path A or C, warm pool stays and is correct (its job is "no white flash," not "content alive")

I'd lean: **(C) + Decision 2** for the next ship — just fix window size, document the content-during-drag limitation honestly. Then revisit Path A vs B as a deliberate Phase 8 instead of trying to retrofit.

---

## Process retrospective — what to change next time

1. **Add tracing FIRST**, ship FIRST build, get logs before iterating.
2. **Search for existing patterns in the codebase** — `tear_off_hook.rs` already had `GetAncestor(GA_ROOT)`; I should have grepped before writing my own HWND lookup. Two attempts wasted.
3. **Research before implementing for unfamiliar primitives.** "How does Chrome actually do tab tear-off" is a 30-minute research task that would have prevented the warm-pool/content-during-drag mismatch.
4. **Don't ship to user's primary working environment.** A test instance on a sandbox monitor would have surfaced the size issue without disrupting work.
5. **The 9-round PR #567 review cycle was overall productive** but ran past the point where each round produced new findings (rounds 7-9 were mostly bot replays). Future PRs: cap at 5 rounds, then merge or pause for direction.

---

## Sources

- [tab_drag_controller.cc on source.chromium.org](https://source.chromium.org/chromium/chromium/src/+/main:chrome/browser/ui/views/tabs/tab_drag_controller.cc)
- [Tab Strip Design (Mac) — chromium.org design doc](https://www.chromium.org/developers/design-documents/tab-strip-mac/)
- [Implementing fallback tab dragging for Wayland — Igalia / Max Ihlenfeldt](https://blogs.igalia.com/max/fallback-tab-dragging/)
- [Wayland Tab Dragging — techhenzy](https://techhenzy.com/wayland-tab-dragging/)
- [Issue 1156893008: Fixes tab dragging out of a window with maximized bounds](https://codereview.chromium.org/1156893008)
- [chromium-tabs Mac port: CTTabStripDragController.m](https://github.com/rsms/chromium-tabs/blob/master/src/Tab%20Strip/CTTabStripDragController.m)
- [Implementing Chrome-Style Tab Tear-off in WinUI 3 — dev.to](https://dev.to/nwlsrb/implementing-chrome-style-tab-tear-off-in-winui-3-3k3j)
- [CEF Forum: CefClient + tab support](https://magpcss.org/ceforum/viewtopic.php?f=6&t=17470)
- [Electron #42054 — WebContentsView popup handling](https://github.com/electron/electron/issues/42054)
- [VSCode #53984 — drag tab to new window](https://github.com/microsoft/vscode/issues/53984)
- HWNDMessageHandler::RunMoveLoop in `ui/views/win/hwnd_message_handler.cc` — `SendMessage(hwnd, WM_SYSCOMMAND, SC_MOVE | 0x0002, GetMessagePos())`
