# Window resize: why AgentMux shows grey before the new size paints, and what it takes to keep up like Chrome

**Date:** 2026-10-06
**Status:** analysis. Measurements in §2, causes in §3, recommendations in §5. R1, R3 and the System Info half of R6 are implemented in the PR that adds this doc, which also covers R2 (see §9 for the results). R4, R5, the rest of R6, R7 and R8 are not yet.
**Author:** Agent4
**Trigger:** Repo owner, 2026-10-06: *"in chrome, if I resize the app window the paint is always tight against the window edge, but in agentmux there is a long delay lag where a grey placeholder appears before the paint makes it. We did work on removing this, I believe there was some sort of debounce. Is that still there? We want the resize of window to make the contents repaint seamlessly, ultra-high performance. ID any bottlenecks in the path."* Later: *"i dragged around in your dev instance, it's definitely an improvement"*.
**Related:** `SPEC_WINDOW_RESIZE_NO_PAINT_DELAY_2026_09_24.md` (proposed, never implemented; its delays are re-checked in §4), `ANALYSIS_WINDOW_TAB_SWITCH_SMOOTHNESS_2026_09_24.md` and `ANALYSIS_WINDOW_TAB_SWITCH_PAINT_2026_09_30.md` (where `window:keepinactivetabslaidout` came from), `SPEC_PANE_REFLOW_ANIMATION_2026_05_29.md`.

Code citations are against `main` @ `07fe1402a`.

---

## 1. Summary

- **The debounce you remember is gone.** #1735 / #1737 (2026-06-23) removed the 50 ms debounce on the tile layout's container resize, and it hasn't come back (`layoutModelHooks.ts:106`). Smaller trailing debounces are still in terminals and charts (§4). They make those panes snap late, but they aren't what makes the whole window lag.
- **The native host isn't the bottleneck.** The main window is a CEF Views window, and Chromium's own `HWNDMessageHandler` sizes it. We add no timer, `WasResized` or off-screen step. In the trace, GPU, compositor and Viz work during a resize is small (§2.3).
- **The bottleneck is the renderer's main thread.** It spends nearly every frame on style recalculation and layout. That happens because every window tab, not just the visible one, is kept laid out (`window:keepinactivetabslaidout`, on by default since #3686/#3687), and each tab's tile layout runs its own resize pass, which forces a synchronous layout.
  - Example: with 4 window tabs of 4 terminals each, a 1.3 s stepped resize produced **30 frames instead of ~96**, with gaps up to **250 ms** and **1.3 to 1.5 s of long frames**.
  - With inactive tabs not laid out, the same resize ran at **60 fps with no long frames**.
- **Why grey.** As far as I know, Chromium fills newly exposed area with the page's background colour until the renderer delivers a frame at the new size. The page background is `#222` (`index.html`, `--main-bg-color`), so that grey band is how long the renderer takes to produce the next frame. I didn't sample the colour during a real drag. Chrome is tight against the edge because its renderer finishes that frame within a vsync.
- **Why the dev window felt better.** It had one tab and almost nothing open. The installed app has many tabs, and today each tab adds a full layout to every resize frame.

## 2. Measurements

All on Agent4's isolated `task dev` window (Windows, 2142×1601 physical px, DPR 1.25, default settings, transparency off). The stepped resize is `SetWindowPos` shrinking then growing the width by 10 px every 16 ms, 80 steps over ~1.3 s, like a drag. The page recorded every `requestAnimationFrame` and every long-animation-frame (LoAF) entry, and one run was traced with `devtools.timeline`, `blink`, `cc`, `gpu` and `viz`.

`SetWindowPos` here doesn't go through Windows' modal size loop, so the per-`WM_SIZING` host work in §3.4 is **not** included. A real drag costs more than these numbers.

### 2.1 Frames during the stepped resize

| State | Resize events seen | Frames (ideal ~96) | Frame gap p50 / p90 / max | Long frames | Long-frame total | of which style + layout |
|---|---|---|---|---|---|---|
| 1 tab, ~21 tiles, no terminals | 38 | 73 | 16.7 / 33.4 / 83 ms | 4 | 248 ms | 109 ms |
| 5 tabs, 16 terminals, inactive tabs laid out (default) | 8–10 | 30–34 | 16.7 / 117–183 / 250 ms | 8–11 | 1,333–1,517 ms | 746–818 ms |
| same, `window:keepinactivetabslaidout: false` | 63–65 | 92–96 | 16.7 / 16.7 / 33 ms | 0 | 0 | 0 |
| same as the default, plus experiment E1 (§2.4) | 9 | 30–33 | 16.7 / 150–200 / 250 ms | 9–10 | 1,323–1,518 ms | 728–868 ms |

"Resize events seen" collapses from 80 to 8 because the main thread is busy for most of the drag, and Chromium coalesces the resizes it can't keep up with. A user sees this as a window that jumps in large steps, with grey filling the gap between them.

### 2.2 Which scripts the long frames blame (LoAF), default state

| Script | Time in 1.3 s | of which forced style + layout |
|---|---|---|
| `ResizeObserverCallback` in `hook/useDimensions.tsx` (the tile layout's container observer, one per window tab) | 476–535 ms | 427–487 ms |
| `setInterval` in `sysinfo/sysinfo-view.tsx` (chart refresh every 2 s) | 47–86 ms | 39–73 ms |

### 2.3 Trace of the same resize (renderer main thread)

| Event | Total over ~1.8 s | Count | Max |
|---|---|---|---|
| `LocalFrameView::UpdateStyleAndLayout` | 1,295 ms | 2,163 | 150 ms |
| `UpdateLayoutTree` (style recalc) | 809 ms | 220 | 75 ms. Average 531 elements per recalc, **max 8,833 (the whole document)**. 14 recalcs over 2,000 elements took 480 ms. |
| `Blink.ForcedStyleAndLayout` | 673 ms | **2,098** | 77 ms |
| `LocalFrameView::NotifyResizeObservers` | 542 ms | 163 | 85 ms |
| `Layout` | 463 ms | 72 | 142 ms |
| `PrePaint` / `Paint` | 108 / 60 ms | | under 14 ms |
| GPU main, Viz, compositor threads | 66–112 ms each | | under 10 ms |

Forced layouts, by the script that caused them:

| Script | Time | Count |
|---|---|---|
| `hook/useDimensions.tsx`: tile layout `updateTree` → `getBoundingClientRect` | 445 ms | 35 (~13 ms each, up to 140 ms) |
| `sysinfo/sysinfo-view.tsx` chart refresh | 144 ms | 90 |
| `AgentComposerStrip.tsx` measure | 7 ms | 546 (cheap, but very frequent) |
| `AgentDocumentVirtualList.tsx` observers | about 3 ms | ~1,100 |

So, as a share of the total: compositing and GPU are under 10%, and paint about 10%. Style and layout on the main thread are 75% or more.

### 2.4 Experiment E1: take the size from the ResizeObserver instead of forcing a layout

`useOnResize` already hands the callback `entry.contentRect`, but `onContainerResize` ignores it, and `updateTree` calls `getBoundingClientRect()` (`layoutGeometry.ts:243`, `:759`).

For this experiment I let the observer's rect stand in for that read, keeping inactive tabs laid out. It was a temporary change in the dev window only, and has been reverted.

- **What changed:** the `useDimensions` callback dropped from ~500 ms to **7 ms**, with no forced layout left in it.
- **What didn't:** frames stayed at 30–33, with 1.3–1.5 s of long frames. The layout work moved into the frame's own layout phase, and the System Info timer forced it instead.

So the forced read is a symptom, not the cost. The cost is laying out every tab's content at the new width on every frame. Experiment E1 is still worth doing (§5 R3), but alone it doesn't fix the lag.

## 3. Causes, ranked

### 3.1 Every window tab relays out on every resize frame (the main cause)

- **The setting:** `window:keepinactivetabslaidout` is on unless set to `false` (`frontend/app/workspace/window-tab-visibility.ts:18-26`). Inactive tabs are hidden with `visibility: hidden; opacity: 0` and stay `content-visibility: visible`. That makes a tab switch one frame (`ANALYSIS_WINDOW_TAB_SWITCH_PAINT_2026_09_30.md`).
- **The cost on resize:** every tab is sized by the window, so each width change invalidates layout and container-query styles in every tab. Every tab's own ResizeObservers fire too: the tile layout, terminals, agent document lists and tab strips.
- **How it scales:** cost grows with the total number of panes in the window, not with what's on screen. Most users who leave tabs open will hit it, and it is the one setting that took the measured case from 30 to 96 frames.

### 3.2 One forced full-document layout per tab per frame

`useOnResize` → `onContainerResize` → `updateTree()` → `getBoundingRect()` → `getBoundingClientRect()` on the tab's container (`layoutResize.ts:266-270`, `layoutGeometry.ts:231-315`, `:759-761`).

- **Why it forces a layout:** each tab's `updateTree` writes new pane transforms and sizes through signals, which Solid applies synchronously. The next tab's read then forces a fresh style recalc and layout of the whole document.
- **The scale:** with N tabs that's N forced layouts per frame. It is 445 ms of the trace.
- **Also redundant:** `updateTree` re-balances the whole tree (`balanceTree=true`) on every tick.

### 3.3 Document-wide style recalculation on resize

14 style recalcs covered 2,000 to 8,833 elements. Candidates, which a trace with `disabled-by-default-devtools.timeline.invalidationTracking` would confirm:
- **Container queries:** settings, section panes, Stash, agent strips, the workspace. Each container that changes size restyles its subtree, and with every tab laid out, that's every tab's subtree.
- **`isContainerResizing`:** toggled per tab at the start and 30 ms after the end (`layoutResize.ts:266-278`). It adds and removes `.animate` on tile nodes.
- **Body layer:** `body { transform: translateZ(0); backface-visibility: hidden }` (`app.scss:38-39`) makes the whole page one compositor layer. Per-pane `will-change: transform` layers (`block.scss:272`, `PaneChrome.scss:64`) and backdrop blurs in tab strips add raster area as the window grows.

### 3.4 Host-side work on every `WM_SIZING` in a real drag

These aren't in the §2 numbers.

- **A page event and a forced layout per tick:**
  - `wndproc.rs:576-600` sends `windowresize:tick` on every `WM_SIZING`, through `ExecuteJavaScript` (`events.rs:20-31`).
  - That's one renderer task per tick, competing with the frame the user is waiting for.
  - The handler (`windowEdgeResize.ts:331-345`) calls `getBoundingClientRect` on every tick even when Shift isn't held, which forces another layout.
- **Position tracking on the UI thread:**
  - The WinEvent hook (`wrr/win_event.rs:871-875`) logs at INFO for every window-position event, before its own filter.
  - Every report then spawns an OS thread that sleeps 1,500 ms (`commands/window/position_persist.rs:76-97`), about 20 per second during a drag.
  - It's all on the CEF UI thread, which also runs Chromium's sizing.

### 3.5 Panes that snap late because of trailing debounces

These are covered by `SPEC_WINDOW_RESIZE_NO_PAINT_DELAY_2026_09_24.md`; §4 below gives their current state. While the window is moving, a terminal or chart keeps its old size and the pane's own background shows around it. That is a second, smaller "grey" inside the panes.

### 3.6 Not causes, checked

- **Transparency.** Off in the installed app's settings and in all the dev channels I checked (`window:transparent: false`, `window:opacity: 1.0`). So the layered-window (`WS_EX_LAYERED`) and transparent-background paths aren't in play. The installed app's default background colour of `0xFF000000` would fill black, not grey.
- **GPU.** Hardware compositing is active, and GPU and Viz threads stay under 10 ms per task.
- **Native message handling.** No `WM_SIZE` or `WM_ERASEBKGND` handling and no `SetWindowPos` on the main window's resize path. `SetWindowPos` returned in 2–5 ms in every run.

## 4. The debounces you asked about

| Delay | Where | State |
|---|---|---|
| 50 ms trailing debounce on the tile layout's container resize | `layoutModelHooks.ts` | **Removed** in #1735 / #1737, 2026-06-23. Still removed. |
| Pane reflow settle window, 220 → 32 → 4 ms | `platform/pane-anim.ts` | Cut in #1531 / #1699. Still 4 ms. |
| Terminal refit + PTY size, `debounce(50)` trailing | `view/term/termwrap.ts:132`, used by `term.tsx:213-215` and `AgentShellSubblock.tsx:734` | **Still there.** The grid stays at its old size for the whole drag, then snaps. |
| System Info chart, 150 ms trailing `setTimeout` | `view/sysinfo/sysinfo-plot.tsx:57-72` | **Still there.** |
| Agent row height cap, 150 ms after `window resize` | `AgentDocumentVirtualList.tsx:724-730` | Still there; affects measurement, not paint. |
| Agent PTY width, 150 ms | `view/agent/hooks/usePtyWidth.ts:53` | Intentional and fine: it delays only the PTY message. |
| `isContainerResizing` reset, `debounce(30)` | `layoutResize.ts:275-278` | Harmless: only re-enables animation. |
| App background overlay IPC, `debounce(30)` | `app-bg.tsx:42` | Harmless: IPC only. |

## 5. Recommendations

In order of impact. Each can be checked with the §6 probes before and after.

- **R1. Don't lay out inactive tabs while the window is resizing (largest win, one change).**
  - **What:** keep `window:keepinactivetabslaidout` for tab switching, but make inactive tabs `content-visibility: hidden` while the window is resizing.
  - **When it's resizing:** from the host's `windowresize:begin`, and from the first container resize tick for resizes without `WM_SIZING`, such as maximize or snap. Release on `windowresize:end`, or 100 ms after the last tick.
  - **After release:** let the inactive tabs relay out once, spread over idle frames (`requestIdleCallback`, one tab at a time), so a tab switch right after a resize is still warm.
  - **What it fixes:** this is the measured 30 → 96 frames.
  - **Alternative:** freeze each inactive tab's container at its pre-resize pixel size (`contain: strict` with explicit width and height) and resize it on release. That avoids the `content-visibility` toggle but needs the same release step.
- **R2. Skip inactive tabs' tile `updateTree` during a resize.**
  - **What:** mark them dirty and run `updateTree` when the tab is next shown or the resize ends. Only the visible tab's tiles need new rects each frame.
  - **Where:** this belongs in the layout model, not in each caller. It also removes N−1 of the per-frame forced layouts in §3.2.
- **R3. Use the observer's size, never a forced read, in `onContainerResize`.**
  - **What:** pass `entry.contentRect`, or better `borderBoxSize`, because `getBoundingRect` today returns the border box. Use it through `updateTree` (experiment E1).
  - **Rule:** keep `getBoundingClientRect` for callers outside an observer callback only.
  - **Also:** don't re-balance the tree on a pure size change (`updateTree(false)`). Re-balancing belongs to tree edits, not resizes.
- **R4. Make the per-tick host event cheap.**
  - **What:** only send `windowresize:tick` when Shift is held, or when its state changes. `windowEdgeResize.ts` only acts on it under Shift. Then remove the unconditional `getBoundingRect()` at `windowEdgeResize.ts:336`.
  - **Payoff:** that takes one renderer task and one forced layout off every frame of a real drag.
- **R5. Take position persistence off the UI thread's per-event path.**
  - Drop the WinEvent INFO log to `debug!` or `trace!`, and move it after the class filter.
  - Replace "spawn a thread per report" in `position_persist.rs` with one long-lived debouncer: a single thread or tokio task with a generation counter. It's the same behaviour without ~20 thread spawns a second.
- **R6. Finish the 09-24 spec.**
  - Refit the terminal grid every frame and debounce only the PTY size. Remove the System Info 150 ms timer, and make the chart's 2 s refresh skip while resizing.
  - Delete the dead `.tile-node.resizing` CSS.
- **R7. Match Chrome's fill colour.**
  - **What:** set the main window's `BrowserSettings.background_color` (`app/mod.rs:1106`) and the Views background to the theme's `--main-bg-color` at window creation. Secondary windows (`ui_tasks/window.rs:1639-1656`) are transparent even when transparency is off, so set them the same way.
  - **Effect:** any frame that does lag fills with the app's own colour.
  - **Why it's last:** it's cosmetic once R1–R4 make the frame arrive in time. It's still worth doing so that one late frame is invisible rather than a band.
- **R8. Find what restyles the whole document, then scope it.**
  - Take one invalidation-tracking trace. If container queries are the cause, R1 already removes inactive tabs from the picture. If it's the `isContainerResizing` class flip, set it on the tab's tile container rather than on every tile node.

**What I'd expect from R1 + R2 + R3 + R4:** the visible tab's own work stays near the 1-tab row of §2.1 (p90 33 ms), however many tabs are open. Getting that last row to a steady 16.7 ms is §3.3 and R8.

## 6. DRY and modularization opportunities found on this path

The repo owner asked for these to be taken as they come up.

- **One resize scheduler instead of per-feature timers.** Terminals (`termwrap.ts`), System Info (`sysinfo-plot.tsx`), the agent list (`AgentDocumentVirtualList.tsx`) and `app-bg.tsx` each roll their own trailing debounce. Each one re-decides, sometimes wrongly, what may wait.
  - **Proposal:** a shared `frontend/app/platform/resize-scheduler.ts` with two lanes. `onFrame(fn)` is for anything that changes pixels, and runs every frame. `onSettle(fn, ms)` is for side effects such as PTY sizes, persistence and IPC, and is flushed on settle, hide and unmount.
  - It should also hold the single "window is resizing" signal that R1 and R2 need. Today that state is spread across `isContainerResizing` per layout model, `windowEdgeResize.ts`'s session and the host's begin/end events.
  - This is the rule §4.5 of the 09-24 spec asks for, made into code.
- **One `ResizeObserver` hub.** About 25 components create their own observer: `useOnResize`, `use-pane-rect-sync`, `PaneTabStrip`, `Tabs`, the widget bar, the launcher, Files, Drone, the agent list's five, and others. A shared observer delivering `{ contentRect, borderBoxSize }` would avoid N observer instances, and give one place to apply "skip while hidden / while the tab is inactive".
  - `hook/useDimensions.tsx` already has the shape of that hub, and only the tile layout uses it today.
- **One source for the container size.** `getBoundingRect()` is read in `layoutGeometry.ts`, `layoutMagnify.ts` and three places in `windowEdgeResize.ts`, each forcing layout. The layout model should keep the last observed size (R3) and have those callers read it.
- **Host: one debounced position writer.** `position_persist.rs` (thread per report) and `transparency.rs` (400 ms opacity write-through) implement the same "debounce then write to srv" pattern separately. One small `DebouncedWriter` (a generation counter and one worker) would serve both.
- **Host: one event-emit path with a cost budget.** `emit_window_edge_resize_event` and the WinEvent reporters each call `ExecuteJavaScript` / IPC per native event. Routing them through one emitter that coalesces to at most one event per frame would cap host-to-renderer traffic during drags generally, not just for resize.

## 7. How this was measured (to reproduce or check a fix)

Scripts are in Agent4's workspace and aren't part of the repo; they can move to `scripts/perf/` if useful.
- **`resizesteps.ps1`:** steps the dev window's width with `SetWindowPos`.
- **`frameprobe.mjs`:** records `requestAnimationFrame` timestamps, `resize` events and long-animation-frame entries through CDP, and summarises them.
- **`traceprobe.mjs`:** records a Chromium trace around the resize and totals main-thread events, plus forced layouts attributed by time to the enclosing `FunctionCall`.

Load for the default-state rows: `createTab()` four times, then four `createBlock({ meta: { view: "term" } })` each, giving 16 WebGL terminals and 8,492 elements.

A check for any fix: the 5-tab, 16-terminal row of §2.1 should get to within a few frames of the `keepinactivetabslaidout: false` row, without turning that setting off.

## 8. Open questions

- Which elements make the 8,833-element style recalcs (R8)? One invalidation-tracking trace answers it.
- Is the grey the page's `#222`, or Chromium's gutter colour? A real-drag screen capture would settle it; R7 makes the answer moot.
- Does the first-tick `notifyPaneReflow()` on Windows (`TileLayout.core.tsx:297-304`) start browser-pane rect sampling during every window resize? It's cheap without browser panes, but should be checked with one open.

## 9. Results: R1 + R3 + the System Info part of R6

Implemented in the PR that adds this doc.

**`frontend/app/platform/window-resize.ts`** holds a `windowResizing()` signal.
- It turns on at the first `resize` event, which fires once per frame for a drag, a maximize, a snap or a programmatic resize, and before that frame's style and layout.
- It turns off `WINDOW_RESIZE_SETTLE_MS` (120 ms) after the last one.
- The same file has `drainWhenIdle`.

**`workspace.tsx`** wires it up:
- When a resize starts, it marks every window tab "relayout pending". `tabContainerVisibility` then gives a hidden tab that's kept laid out `content-visibility: hidden` instead of `visible`, so the browser skips its layout, style and ResizeObservers. The displayed tab is never affected.
- When the resize settles, the pending tabs are released one per idle period, so they catch up without one long frame. That release order also covers R2: a tab skipped this way never runs its tile `updateTree` during the resize.

**`layoutModel.ts`** (R3): `onContainerResize` takes the ResizeObserver entry's `borderBoxSize` (now passed by `useOnResize`) and `getBoundingRect` returns it for that pass instead of calling `getBoundingClientRect()`. In the dev window the two agree to 0.01 px.

**`sysinfo-view.tsx`**: the 2 s chart refresh skips its update while the window is resizing. It was the last script forcing a layout mid-resize.

**Measured**, same setup as §2.1: 5 tabs, 16 terminals, `window:keepinactivetabslaidout` left **on**, three runs:

| | Frames (ideal ~96) | Frame gap p50 / p90 / max | Long frames | Long-frame total |
|---|---|---|---|---|
| Before (§2.1) | 30–34 | 16.7 / 117–183 / 250 ms | 8–11 | 1,333–1,517 ms |
| After | 92–95 | 16.7 / 16.7 / 33 ms | 0–1 | 0–55 ms |

**A tab switch right after a resize** is still warm. The hidden tabs had caught up, all four back to `content-visibility: visible`. The switch's first frame came 21 ms after `setActiveTab`, with no long frames.

**Still open:**
- R4: the host's per-`WM_SIZING` event plus forced layout. It only shows in a real mouse drag.
- R5: the host's per-event logging and thread spawns.
- R6: the terminal refit debounce.
- R7: the fill colour.
- R8: what restyles the whole document.
