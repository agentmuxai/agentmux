# Tear-off latency: the path from pointer down to the torn-off window, and what blocks the page

Audit of the Windows tear-off of a pane (drag a pane's header out of the window). It follows the code from the press to the
floating window painted and the source window settled, and ranks what blocks the page. Requirement behind it: a user-facing
gesture answers as fast as it can, and nothing on its path blocks the page's frames.

Status: research report. Nothing is changed by it. The numbers come from the app's own performance log lines of two real
tear-offs on a very large window (a viewport of roughly 3.7k by 2k CSS px, one pane about 0.9k by 2k). Where a cause is
read from the code and not yet shown by a trace it says so, and section 6 lists what a measured run on a test host has to
confirm before each fix.

## 0. Summary: the three biggest costs

1. **A full-window screenshot on every press of a pane header, handled by the page.** `picturePaneOnPress` takes a picture of
   the whole window at pointer down for any press on a draggable header, whether or not a drag follows
   (`frontend/app/drag/CrossWindowDragMonitor.win32.tsx`, `tearoff-snapshot.ts`). The host takes it with a browser-protocol
   screenshot (0.1 to 1.3 s in the logs), sends it to the page as base64, and the page decodes it, crops it, encodes it
   again and sends it back to the host as base64. In one logged tear-off the page produced a single 886 ms frame holding
   10 ms of script while that screenshot ran, and the picture was ready 1.7 s after the press. Fix: the host takes, crops
   and keeps the picture itself (the page only says which rectangle), and a press that is only a click stops paying for it.
2. **Forced layout at drag start.** The drag-start listener of the layout's drag library took 264 ms in one frame, 251 ms of
   it forced layout, followed by a 77 ms animation callback with 69 ms forced layout: 720 ms in one frame, 608 ms of it
   blocking. The code reads geometry (`getBoundingClientRect`, `elementFromPoint`) at that moment while styles are dirty.
   Fix: find the write that dirtied the page (section 2.2) and keep reads before writes, and limit how much of the window a
   pane's style change can re-lay out.
3. **Serial host calls on a page that is still busy.** The release is a chain of about seven awaited calls
   (`getCursorScreenPoint`, `startCrossDrag`, `updateCrossDrag`, `TearOffBlock`, a diagnostic `getPaneDebugState`,
   `openFloatingPane`, `completeCrossDrag`, `releaseDragCapture`), and each waits 40 to 90 ms for the page to take the
   answer although the host answered in 0 to 1 ms. Fix: fewer calls (the host already knows the cursor), the diagnostic
   off the critical path, calls that do not depend on each other started together, and, above all, a page that is not
   blocked when the answers arrive (costs 1 and 2).

## 1. The path, step by step

Windows, a pane torn off by its header. Marks: **N** needed before the person sees feedback, **A** can come after.
Thread names: page = the window's renderer main thread; host = the host process (crates/cef); srv = the backend.

| # | Step | Where | Thread | Waits on | Mark |
|---|------|-------|--------|----------|------|
| 1 | Pointer down on a pane header | `picturePaneOnPress` | page | nothing | the press itself N |
| 2 | Rects read, `beginPaneCapture` (the drag's own drawing is held back, up to 400 ms) | `prewarmTearOffSnapshot` | page | style and layout (forced if dirty) | read N, hold A |
| 3 | Window screenshot, JPEG q80, whole viewport | `windows.captureViewport` -> `capture_window_viewport` (browser-protocol `Page.captureScreenshot`, `fromSurface`) | host + renderer | the renderer producing a frame | A (see 2.1) |
| 4 | The picture's base64 comes to the page, is decoded, cropped to the pane on an offscreen canvas, encoded to JPEG, to base64 | `cropToPane` / `composeBrowserPane` | page | the page's main thread being free | A |
| 5 | Drag starts: the library's `dragstart` listener runs `canDrag` (`rejectDragAt` reads the container rect, `elementFromPoint`), `onGenerateDragPreview` (reads the tile and header rects), then `onDragStart` in the next frame (`startTileDrag`, `setIsDragging`) | `TileLayout.core.tsx`, `TileLayout.win32.tsx` | page | layout | feedback N, the reads can be before |
| 6 | Host side of the drag: `start_tab_drag_tracking` (mouse hook thread) | `tear_off_hook/windows.rs` | host | page taking the answer (85 ms back) | A |
| 7 | The picture is handed to the host: `followPaneDrag` -> `liveFollowBegin` (base64 again) | `pane-live-follow.ts` | page, host | the picture (step 4), the page being free (388 ms back in one log) | the picture shows with the cursor: N for the hand |
| 8 | Per cursor move, the host's hook moves the picture; the page is not on this path | `live_follow.rs` | host | nothing of the page | good as is |
| 9 | Release: `getCursorScreenPoint`, `startCrossDrag`, `updateCrossDrag` in sequence | `CrossWindowDragMonitor.win32.tsx` | page <-> host | each answer taken by the page | N (one call would do) |
| 10 | `TearOffBlock` (srv saga) runs with the picture's own wait (`takeTearOffSnapshot`) | `WorkspaceService.TearOffBlock` | srv | the backend, about 100 to 190 ms | N: it moves the pane into a new workspace |
| 11 | `getPaneDebugState`, awaited for a log line only | same file | page <-> host | the page | **A: not needed before the window** |
| 12 | `openFloatingPane` -> promote a warm pool window, show it after its snapshot, front | `floating_pane.rs`, pool | host | 20 to 30 ms | N |
| 13 | `completeCrossDrag`, `releaseDragCapture`, the layout drops the pane (`removeTornOffPane`) | same file | page <-> host | the page | A (the window is already up) |
| 14 | The new window starts (about 70 ms of startup work in the page) and asks the host about the other windows: `list_window_instances` about ten times, `register_backend_window` twice, `set_window_opacity`, `set_window_init_status`; each answer waits 180 to 290 ms | `initCefNewWindow`, `cef-api.ts` | new page <-> host | the new page being busy | A |
| 15 | The source window refills the pool and settles (layout reflow, animation) | `pane-pool`, layout | host, page | | A |

What the person sees: the window has a picture that follows the cursor (step 8) only after step 7, so everything before
it (steps 2 to 7) is the gap between starting a drag and the first feedback. In the two logged tear-offs the picture was ready about
0.35 s and 0.8 s after the drag started (1.1 s and 1.7 s after the press), and the floating window was up 1.3 s and
0.8 s after the drag started.

## 2. Each cost: cause and fix

### 2.1 The picture (steps 2 to 4 and 7)

**Cause, in the code.**
- It starts at pointer down for every press on a draggable header (not a control), because the picture has to show the pane
  as it stands before the drag draws on it. A click that never drags pays the same: a press with no drag after it logged a 442 ms capture.
- The screenshot is the browser-protocol `Page.captureScreenshot` with `fromSurface`, for the whole viewport, JPEG q80.
  The host log shows it at 0.1 s to 1.3 s (a 1.27 s run, then 0.37 s, then 2.5 s at a re-dock seconds later), with the
  page's frames held (an 886 ms frame containing only 10 ms of script) in the same second. The likely mechanism is that
  taking a surface screenshot stalls or waits on the renderer's compositor; this report does not prove it (6.1 does).
- The result then crosses the page twice as base64 (megabytes of text for a window this size), is decoded
  by the page (`createImageBitmap`), drawn to an offscreen canvas, encoded again and sent back. All of it is main-thread
  or main-thread-scheduled work in the page that is also trying to start a drag.
- The drag's own drawing waits for the capture (`pane-capture-gate.ts`, up to 400 ms), so a slow capture delays the first
  visible response of the drag by up to 400 ms.
- The host already has the better shape for the landing picture: `live_follow_capture` takes the window picture and cuts
  the pane out in the host ("so the page does none of the work"), and keeps it there.

**Fix, in order.**
1. Take the first picture the same way: a host call that captures, crops the pane rectangle (given in CSS px and the
   viewport width, as `live_follow_capture` already takes) and keeps it for the drag; the page never receives or sends the
   picture's bytes. `takeTearOffSnapshot` for the floating window's first paint uses the same held picture.
2. A press that is only a click: start the capture on drag intent (the first move past the drag threshold, or the library's
   drag start) and not on pointer down, or cancel it on pointer up without a drag, so a click costs nothing. The drag's
   drawing hold (`pane-capture-gate`) is kept, but with the capture in the host and not stalling the page its wait
   falls to the capture's own time.
3. If 6.1 shows the capture itself stalls the page's frames, take it natively in the host (the window's own pixels, off the
   page's threads) instead of the browser-protocol screenshot, with the browser-protocol one as the fallback.
4. Keep the quality and scale a setting of the host call so the picture can be taken at the monitor's own scale and not larger.

### 2.2 Forced layout at drag start (step 5)

**Cause, in the code.** The pointer-down path writes style (`beginPaneCapture` flips a signal the layout reads;
the press focuses and selects the pane), then within one frame the drag library's `dragstart` listener calls
`canDrag`, whose `rejectDragAt` reads `getBoundingClientRect` on the container and whose body calls
`document.elementFromPoint`, then `onGenerateDragPreview` reads the tile and header rects. The first read after a write
pays the whole document's style and layout. The log attributes 251 ms of the listener to forced layout and another
69 ms to the next frame's `onDragStart` callback, so one write-then-read pair costs most of a second on a window this size.
The pane's whole subtree and the window's other panes are in the same layout, so one class flip on one pane lays out all.

**Fix.**
- Read first, write second: gather the container rect, the header and tile rects and the element under the pointer in one
  read phase at pointer down (the press already reads the pane rect), keep them, and use them in `canDrag` and
  `onGenerateDragPreview`, so the listener reads nothing live.
- Move the writes that follow a press (the capture gate's signal, focus and selection restyle) after the drag has
  started, or batch them in the same frame as `onDragStart`.
- Contain a pane's layout and style (`contain: layout style` or `content-visibility` on the leaf's body) so a change inside
  one pane cannot re-lay out the window. Check against the animated reflow and the browser panes' placeholders first.
- A trace of the press to the first drag frame (6.2) names the write that dirtied the page; do that before choosing among these.

### 2.3 Serial calls on a busy page (steps 9, 11, 13) and the second window (step 14)

**Cause.** `handleCrossWindowDragEnd` awaits the calls one after another, and each answer waits for the page to take it
(the host's own time is 0 to 1 ms). `getPaneDebugState` is awaited only to write one log line. In the new window,
`list_window_instances` is asked about ten times in the first second, and each answer waits up to 290 ms because the new
window's page is busy starting up.

**Fix.**
- One host call for the release (`getCursorScreenPoint` + `startCrossDrag` + `updateCrossDrag`): the host has the cursor.
- Take `getPaneDebugState` out of the critical path (fire it after `openFloatingPane` returns, or drop it).
- `TearOffBlock` and the host's pool promotion do not depend on each other except for the workspace id; start the
  promotion's preparation with the call and have `openFloatingPane` take the id when it is known.
- `completeCrossDrag` and `releaseDragCapture` can follow without being awaited by the drop handler.
- The new window asks `list_window_instances` once and shares the answer (or the host pushes it); `register_backend_window` once.
- The repeated stalls on the way back disappear for the calls whose cost was only the page being busy once costs 1 and 2
  are fixed: after the tear-off the call to open the floating window answered in 5 ms back when the page was idle.

### 2.4 Smaller costs on the path

- `workspace.TearOffBlock` 102 to 193 ms (srv): needed before the window shows its content; look at what it waits on after
  the rest is fixed.
- `live_follow_begin` 112 to 452 ms, almost all "back" (the page taking the answer): disappears with 2.1.1.
- 60 ms of `Response.json.then` work in the page during the drag: a response handler that parses on the main thread; see 4.

## 3. A budget

Proposed, as numbers:
- Something visible within **100 ms** of the gesture: the pane's picture (or the drag preview) under the cursor, or the
  pane's own lifted state, at the first move past the drag threshold.
- No blocked frame over **50 ms** on a gesture path (pointer down to the end of the drop), and no frame over **16 ms** of
  script of ours while the cursor is moving.
- The floating window painted within **250 ms** of the release.
- Every host call on the gesture path answered within **16 ms** when the page is idle.

What the app already logs to check it: `[perf] long-frame <ms> (blocking <ms>)` with the listener that held it, forced
layout included; `[perf] long-task`; `[perf] ipc <name> <ms> (to host, host, back)` for any call over 25 ms, which
separates the host's time from the page's; `[perf] slow-interaction` for input delay and presentation; and
`[tearoff-perf] snapshot shown <ms> after promote`, `tear-off snapshot captured ms`, `[live-follow] picture ready`,
`[pane-pool] floater shown in front` (the end of the path). A check that fails the build is a unit test that counts the geometry reads
the drag start makes and the host calls the release awaits (a ceiling on each), plus a logged check in a real run: the
dragstart frame's `blocking` against the 50 ms budget.

## 4. Other gesture paths the same log shows blocking (ranked, no fixes here)

1. A WebSocket message handler in `mps` taking 36 to 100 ms with forced layout, in over four hundred long frames in one day:
   the busiest single source, running during drags too.
2. The animation callbacks of the swarm list and of the window's own frame loop (hundreds of long frames each).
3. A resize-observer callback in the magnified-size code, about 150 long frames.
4. The drop listener (`ondrop`) on the layout: 68 to 78 ms with 22 to 69 ms forced layout at each drop.
5. Wheel listeners (strip wheel, the window's `onwheel`): 28 to 37 ms of forced layout on every tick.
6. Document `click` and `focusin` listeners taking 16 to 54 ms.
7. `Response.json.then` handlers (the page parses a response on the main thread) at 50 to 60 ms.
8. A window-start import (`import.then`) of 146 ms in a new window after a tear-off.

## 5. What differs on macOS and Linux

The picture path above is Windows only: `picturePaneOnPress`, `prewarmTearOffSnapshot`, `live_follow` and
`ensureTabPicture` are Windows code, and the macOS and Linux monitors do not take a screenshot at the press, so the cost in
2.1 does not exist there. They share the drag library's listener (2.2) and the serial release calls (2.3) in their own
form: macOS drives the drag with the system's drop source and `tear_off_hook/macos.rs`, Linux with the browser's own drag
image (a different drag-image path on Wayland). The same budget applies; the fixes in 2.2 and 2.3 are shared code. One
drag each on the fix branch from the macOS and Linux testers is part of the plan before the first merge.

## 6. What the next measured run has to show

On a test host (not the operator's machine), with a task dev build of current main and a real drag, reading the log lines above:
1. **Does the screenshot stall the page?** The page's frame times while a capture runs with no drag, a trace of the
   renderer and the capture's own time as the window size grows. It decides whether 2.1.3 is needed.
2. **Which write dirties the page before `dragstart`?** A performance trace from pointer down to the first drag frame with the
   forced-layout stack, before and after moving the reads.
3. **The release's call chain with the page idle:** the same drop with the picture work removed, to see the floor (the
   "back" of each call).
4. The baseline numbers for the budget (the dragstart frame, the press-to-feedback time, the release-to-window time) at two
   window sizes and two monitor scales, which the PRs then quote before and after.

## 7. How it is fixed (the build)

Small PRs, each with before and after numbers from section 6:
1. Host-side first picture and cropping, no page round trip; clicks stop capturing (2.1.1, 2.1.2).
2. Reads before writes at drag start, and a count of forced layouts in a test that fails over a ceiling (2.2).
3. The release's call chain: fewer, started together, the diagnostic off the path (2.3).
4. The new window's repeated host questions (2.3).
5. A native capture, only if 6.1 shows the screenshot stalls the page.

A fix that changes what the person sees during the drag (a different picture, a different order of what appears) goes to
the requester first with a screenshot or recording. The redock side of these windows has a fix of its own in progress, so
its files are not changed without checking with its author.
