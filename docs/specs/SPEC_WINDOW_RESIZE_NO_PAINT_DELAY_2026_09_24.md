# SPEC — Window resize repaints pane content every frame, with no settle delay

**Status:** proposed
**Date:** 2026-09-24
**Author:** maricon
**Related:**
- #1531, #1699: pane reflow settle window 220 ms → 32 ms → 4 ms (`frontend/app/platform/pane-anim.ts`)
- #1735 / #1737: removed the 50 ms debounce on the tile-layout container resize (`layoutModelHooks.ts`)
- #1667, #1747: sysinfo chart 150 ms resize debounce, then per-instance gradient ids
- #986: agent-pane PTY width 150 ms debounce (`usePtyWidth.ts`)
- #2581: `SPEC_AGENT_SHELL_PSREADLINE_THAW_VISIBLE_RESIZE_2026-08-14.md` (SIGWINCH coalescing and PSReadLine)
- `SPEC_PANE_REFLOW_ANIMATION_2026_05_29.md` §3.2, `SPEC_SYSINFO_CHART_ROBUSTNESS_2026_06_21.md` §Change 4

---

## 1. Problem

During a live window resize (dragging the window edge), pane content lags the
window. Terminal panes are the clearest case: the text grid stays at its old
size for the whole drag and snaps to the new size only after the pointer
stops. Growing the window leaves an empty band on the right and bottom;
shrinking clips text at the edges. It makes rendering look slow, but the
renderer isn't slow. **A timer deliberately holds the repaint back.**

The question asked: is there any reason to delay? **No, not for painting.**
The delay protects the shell behind the terminal from resize storms. That
concern is real, but it belongs on the PTY resize, not on the on-screen grid.

## 2. Where the delays are (main @ `3d0092559`)

### 2.1 The window-resize pipeline itself: no delay

- The native host adds no delay to a plain border-drag resize on any platform.
  The main window is a CEF Views window and Chromium sizes it. There is no
  `WM_SIZE`/`WasResized` handling of our own, no off-screen rendering, and no
  host-side debounce.
- The tile layout reflows every frame: `useOnResize(displayContainerRef,
  onContainerResize)` (`frontend/layout/lib/layoutModelHooks.ts:69`) passes no
  debounce, so `updateTree()` runs on every ResizeObserver tick. This was fixed
  in #1735.
- `createStopContainerResizing` (`frontend/layout/lib/layoutResize.ts:275`,
  `debounce(30)`) only delays switching animations back on. It never delays
  content.

### 2.2 Delays that hold pane content back

| # | Where | Delay | What is held back | Visible mid-drag |
|---|---|---|---|---|
| D1 | `frontend/app/view/term/termwrap.ts:132` `handleResize_debounced = debounce(50, …)`, called by the ResizeObserver at `term.tsx:213-216` | 50 ms, **trailing** | xterm grid refit (`customFit` → `terminal.resize`) **and** the PTY size (`sendTermSize`) | **Yes, the main symptom.** The grid is frozen for the whole drag. |
| D2 | `frontend/app/view/agent/components/AgentShellSubblock.tsx:703-708` (same `handleResize_debounced`) | 50 ms, trailing | Agent Shell drawer terminal refit | Same as D1, inside the drawer |
| D3 | `frontend/app/view/sysinfo/sysinfo-plot.tsx:38-60` `setTimeout(…, 150)` after the first event | 150 ms, trailing | Chart re-render at the new size | Chart stays at its old size, clipped or with blank space |
| D4 | `frontend/app/view/agent/hooks/usePtyWidth.ts:55` `DEBOUNCE_MS = 150` | 150 ms, trailing | PTY column count sent to the backend only | **No repaint effect.** New CLI output wraps at the old width until settle. |

**Why a trailing debounce freezes rather than just delays.** throttle-debounce
v5's `debounce(delay, cb)` defaults to `atBegin: false`: the callback runs
`delay` ms after the **last** call. ResizeObserver fires about once per frame
(~16 ms), which is shorter than 50 ms. So during a continuous drag the timer
keeps being pushed back and never fires until the pointer pauses. D1 isn't
"50 ms late"; it doesn't repaint at all until the drag stops. This is the exact
bug #1735 found and removed for the tile layout ("updateTree() only ran once —
50ms after the user stopped dragging").

### 2.3 Nearby, not part of this change (recorded for completeness)

- **Browser panes** (`use-pane-rect-sync.ts`) trail the DOM by one IPC round
  trip per frame. No timer, and the latency is inherent to the native child
  window.
- **macOS browser panes:** every `browser_pane_resize` posts a 50 ms reaffirm
  that re-applies the rect captured at call time
  (`agentmux-cef/src/ui_tasks/pane_geometry.rs:785-794`, `012d75a6b`). During a
  live resize that rect can be stale, so the pane may briefly jump back. It
  also runs a lot of ObjC work and info-level logging on every tick. Follow-up
  F2.
- **Windows title-bar drag / un-maximize:** the nested move loop plus
  `UNMAXIMIZE_SETTLE_MS = 48` (`agentmux-cef/src/ui_tasks/drag.rs:820`). This is
  a documented, intentional workaround for Chromium deferring work inside a
  nested native loop (`SPEC_WINDOW_SNAP_MAXIMIZE_2026_09_04.md` §2.6). It
  doesn't apply to border-drag resize. Keep it.
- **Resize fill colour:** the CEF `background_color: 0xFF000000`
  (`agentmux-cef/src/app/mod.rs:1040-1047`) is what Chromium paints into
  newly exposed area until the renderer delivers a frame at the new size. When
  growing the window, black edges filling in can read as "slow paint".
  Follow-up F3.
- **Dead CSS:** `.tile-node.resizing { backdrop-filter: blur(8px) }`
  (`frontend/layout/lib/tilelayout.scss:92-95`) is never applied, since nothing
  sets the class. Remove it (§4.4).

## 3. Why each delay was introduced

- **D1/D2, terminal 50 ms.** Inherited from the initial import
  (`4be0e8d4a`, "Initial commit - AgentMux v0.31.20", upstream Wave Terminal
  code). There is no comment, spec or issue explaining it. The standard reason
  for debouncing an xterm fit is below, and it is about the **PTY**, not
  painting:
  1. Every column or row change sends `setblocktermsize` over the WebSocket, and
     the backend forwards it as a real PTY resize (`agentmux-srv/src/server/websocket.rs`,
     `"setblocktermsize"` arm). Each PTY resize sends SIGWINCH, and shells
     (zsh, bash, PSReadLine) redraw their prompt on every SIGWINCH. A per-frame
     SIGWINCH storm during a drag produces duplicated prompts and jumbled
     output. VS Code hit exactly this (microsoft/vscode #330040: 3 prompt copies
     with a 100 ms debounce, 1 with 300 ms).
  2. A column change makes xterm reflow its whole buffer. With large scrollback
     that costs O(buffer) per resize.
- **The history of tuning these values (June 2026).** Every settle or delay on
  the resize path has been shortened or removed once someone measured it:
  - 220 ms → 32 ms (#1531): the window tracked a CSS reflow animation that had
    been removed.
  - 32 ms → 4 ms (#1699): only scheduler slack remained.
  - 50 ms container debounce removed (#1735): it froze native panes for the
    whole drag.

  Nobody found a case that needed the delay once the thing it was protecting
  was gone. The terminal debounce was the one that got missed, because it
  lives in the terminal view rather than the layout.
- **D3, sysinfo 150 ms (#1667).** During dock/undock transitions two SVG copies
  of the chart shared one gradient id, which produced a sliding line artifact.
  The debounce hid it. #1747 then gave every instance a unique gradient id
  (`gradient-${blockId}-${yval}-${++_gradientSeq}`, `sysinfo-plot.tsx:36`), which
  removed the root cause. **The debounce's reason no longer applies**, and the
  code comment is stale.
- **D4, PTY width 150 ms (#986).** It keeps a drag from sending a PTY-resize RPC
  on every frame ("a drag-resize emits at most one RPC per gesture";
  `docs/analysis/AGENT_PANE_PTY_RESIZE_RACE_2026_06_16.md` §4.3). This is the
  right concern in the right place, since it gates only the PTY. Keep it.

## 4. Proposal

### 4.1 Terminal (D1, D2): split the visual refit from the PTY resize

Replace the single trailing debounce with two separate policies:

1. **Visual refit: every frame, no timer.** The ResizeObserver callback calls
   the grid refit (`customFit()`) directly. ResizeObserver already delivers at
   most one notification per frame, after layout and before paint, so no extra
   coalescing is needed; a debounce only adds latency without saving any work.
   The `_renderService.clear()` + `terminal.resize()` pair runs in the same
   task before paint, so it doesn't flash.
2. **Column reflow cost: follow VS Code's `TerminalResizeDebouncer`.**
   - Rows (Y): always immediate. A row change is cheap.
   - Columns (X): immediate when the normal buffer is small (VS Code's
     `StartDebouncingThreshold = 200` lines).
   - For a large buffer, **throttle** the X refit (leading and trailing edge,
     about 100 ms) instead of debouncing it. The grid then keeps tracking the
     drag at roughly 10 Hz instead of freezing.
   - A hidden terminal defers its refit to idle time (VS Code uses
     `runWhenWindowIdle`). Ours are inactive tabs.
3. **PTY size (`sendTermSize`): coalesce, trailing edge, and send only when
   cols/rows changed.** Keep a trailing debounce here, 150 ms to match
   `usePtyWidth`. VS Code moved its equivalent from 100 ms to 300 ms because of
   zsh prompt duplication, so validate with zsh, bash and PowerShell/PSReadLine
   before picking the final value. Always flush the pending send on drag end,
   pane hide and unmount, so the PTY never stays at a stale size.
   - The shell sees its old width until the drag settles. That's fine: xterm
     reflows what's already on screen, and new output uses the correct width
     after the single SIGWINCH.
4. **Unchanged:**
   - Zoom and font-size changes keep calling `handleResize()` directly
     (`term.tsx:265-274`).
   - The #2581 PSReadLine thaw keeps its own two-rAF resize cycle.
   - The `proposeDimensions()` NaN guard stays.
5. **API shape:** replace `handleResize_debounced` with `handleResizeLive()`
   (visual refit now, PTY send coalesced). Both call sites (`term.tsx`,
   `AgentShellSubblock.tsx`) switch to it. `handleResize()` (refit and send now)
   stays for the discrete callers.

### 4.2 Sysinfo chart (D3): drop the timer

Apply the size on every ResizeObserver tick, as the first tick already does,
and delete the 150 ms `setTimeout` along with its stale comment. If rebuilding
the plot every frame measurably costs too much, coalesce to one per
`requestAnimationFrame`. Never use a trailing debounce here.

### 4.3 Agent PTY width (D4): keep

It only gates the backend PTY column count, which is exactly the thing that
should be coalesced. No change, apart from confirming it flushes on unmount.

### 4.4 Cleanup

- Delete the dead `.tile-node.resizing` rule (`tilelayout.scss:92-95`).
- Delete the stale `content-visibility: auto` claim in `_document.scss:133`.

### 4.5 Rule for future work

> A resize handler must never gate **painting** behind a trailing debounce.
> Paint-side work runs on every ResizeObserver tick; coalesce to one per frame
> with `requestAnimationFrame` only when the handler writes layout that the same
> observer watches. Coalescing with a timer is only for **side effects that
> leave the renderer**: IPC/RPC, PTY resize, persistence. Those must flush on
> settle, hide and unmount.

## 5. Best-practice basis

- **ResizeObserver already batches per frame.** It delivers after layout and
  before paint, once per frame, so a manual debounce adds latency without
  removing any work. Coalesce with rAF only to break observer write loops.
  ([MDN ResizeObserver](https://developer.mozilla.org/en-US/docs/Web/API/ResizeObserver),
  [ObserverViewport patterns](https://www.observerviewport.com/implementation-patterns-for-viewport-resize-tracking/),
  [Akulov, "How to optimize resizing or scrolling"](https://iamakulov.com/notes/resize-scroll/))
- **Throttle, not debounce, for continuous feedback.** A trailing debounce runs
  only after input stops, so it can't give live feedback during a gesture. A
  throttle, or rAF coalescing, keeps updating while the gesture is still going.
  ([Go Make Things, debouncing with rAF](https://gomakethings.com/debouncing-events-with-requestanimationframe-for-better-performance/))
- **Terminals separate grid and PTY.** VS Code's `TerminalResizeDebouncer`
  resizes rows immediately, resizes columns immediately for small buffers,
  debounces column reflow only for large buffers, and defers hidden terminals
  to idle time. Its PTY-resize debounce was raised to 300 ms to stop SIGWINCH
  prompt duplication.
  ([terminalResizeDebouncer.ts](https://github.com/microsoft/vscode/blob/main/src/vs/workbench/contrib/terminal/browser/terminalResizeDebouncer.ts),
  [microsoft/vscode #330040](https://github.com/microsoft/vscode/pull/330040))
- **The common "debounce fit() by 150–200 ms" advice is for canvas drift and
  PTY churn, not repaint speed.** It trades live feedback for fewer resizes,
  which is the wrong trade for the on-screen grid.
  ([Kilo-Org/cloud #1195](https://github.com/Kilo-Org/cloud/issues/1195),
  [xtermjs discussion #4436](https://github.com/xtermjs/xterm.js/discussions/4436))
- **Embedded Chromium doesn't guarantee one-frame resizes.** Some trailing is
  inherent, so don't add more on our side.
  ([CEF forum: resize lag](https://www.magpcss.org/ceforum/viewtopic.php?f=10&t=19718))

## 6. Test plan

**Unit tests (vitest), with fake timers and a stub ResizeObserver:**
- A burst of observer ticks every 16 ms for 500 ms calls `terminal.resize`
  **during** the burst: at least once per tick for a small buffer, at least
  every ~100 ms for a large one. It must not only fire once after the burst.
  This test fails on current `main`.
- During the same burst, `sendTermSize` fires **once**, after settle, and only
  if cols/rows changed. Unmounting or hiding mid-burst flushes the pending send.
- Sysinfo: a burst updates `plotWidth`/`plotHeight` on every tick. There is no
  150 ms tail.

**Manual smoke, on Linux, Windows and macOS:**
- Drag a window edge continuously with a terminal pane, the agent Shell drawer
  and sysinfo open. Content should track the drag, with no snap on release.
- Shells: zsh, bash, and PowerShell/PSReadLine on Windows. Do a
  shrink/grow/shrink drag, then check that exactly one prompt remains and the
  PSReadLine cursor is still in sync (regression guard for #1042 and #2581).
- Large scrollback (10k lines): the drag must stay smooth. Measure frame times
  with the perf marks (`@/perf`) before and after.

## 7. Risks

- **Per-frame reflow with a large buffer:** mitigated by the X-throttle in
  §4.1.2. Measure before and after.
- **Stale PTY width during the drag:** new output produced mid-drag wraps at the
  old width until the single SIGWINCH on settle. This matches VS Code, and is
  better than a frozen grid.
- **WebGL renderer canvas reallocation every frame:** xterm resizes the canvas
  on each grid change. Watch GPU memory churn in the large-window smoke.

## 8. Follow-ups (separate PRs)

- **F1:** browser-pane rect sync. Measure the IPC trailing; consider sending the
  rect ahead of layout (already ruled a non-goal in
  `docs/retro/perf-baseline-2026-05-09.md` H1, so revisit only if it is
  measured).
- **F2:** macOS `SetPaneBoundsViewsTask`. Have the 50 ms reaffirm re-read the
  **current** pane rect instead of the captured one, and skip the
  focus/`orderFront` work on repeated resize ticks.
- **F3:** resize fill colour. Set the CEF `background_color` to the theme
  background, so growth fills with the app colour rather than black.
