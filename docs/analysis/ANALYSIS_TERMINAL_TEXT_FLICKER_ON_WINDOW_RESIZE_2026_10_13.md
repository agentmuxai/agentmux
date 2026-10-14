# Terminal text flickers while the window is resized: every grid resize paints one blank frame

**Date:** 2026-10-13
**Status:** analysis; the fix in §5 is implemented in the PR that adds this document.
**Author:** Camper
**Trigger:** Repo owner, 2026-10-13: *"a subtle flickering of the text of the terminal when resizing the window that doesn't appear on other panes … can we get the terminal text to appear solid no matter what? … the flicker is an indication of a bad code path, perhaps poor architecture."*
**Related:**
- `SPEC_WINDOW_RESIZE_NO_PAINT_DELAY_2026_09_24.md` §4.1: live terminal refit during a drag, #4399.
- #4408 and #4658: the per-frame refit scheduler and its time budget.
- `ANALYSIS_WINDOW_RESIZE_REPAINT_LAG_2026_10_06.md`.

Code is cited against `main` @ `298b82096`, with `@xterm/xterm` 6.0.0 and `@xterm/addon-webgl` 0.19.0. xterm's internals are read from the original TypeScript in the packages' source maps.

---

## 1. Summary

- **The flicker is a blank frame, one per grid resize.** Measured frame by frame, every time a terminal's columns or rows changed during a window drag, the frame the browser then painted showed the terminal's canvas empty. In the measured drag that was 15–16 blank frames, out of about 16 grid changes.
- **Why the terminal and no other pane:**
  - xterm draws text into a WebGL canvas, and resizing a canvas empties it.
  - xterm resizes the canvas inside `terminal.resize()`, but only *asks* for the repaint, with `requestAnimationFrame`.
  - We call `resize()` from a ResizeObserver callback, or from an animation-frame callback when a refit is deferred. Either way, the requested repaint runs in the next frame, so the current frame is painted with the empty canvas.
  - DOM panes have no canvas to empty: their text is laid out and painted in the same frame.
- **A second, smaller path:** xterm's own ResizeObserver on the canvas re-sets its size when the device-pixel size comes out different from what xterm set. That's fractional cell widths at a 125% display scale. It empties the canvas again and again defers the repaint, 3–4 times per drag.
- **Fix:** paint the rows synchronously right after each resize, from our refit, and from an observer registered after xterm's for the second path.
- **Measured** in the same session, three drags each:
  - **blank frames on the visible terminal:** 15–16 per drag before, **0** after;
  - **page frame times:** unchanged, p50 16.7 ms both ways, p90 21.9–23.5 ms before and 20.4–21.0 ms after.
- **The architecture point:**
  - **Why it happened:** the bug lives where our code meets xterm's renderer. We copied `FitAddon.fit()`'s clear-then-resize sequence, and that sequence assumes a repaint "soon" is good enough. For a canvas renderer it has to be the same frame.
  - **The fix's catch:** it calls one more xterm internal, `_renderService._renderRows`, beside the internal we already call there (`_renderService.clear`).
  - **The durable fix** belongs in xterm itself: `RenderService.handleResize` should render synchronously when the canvas was reallocated (§6).

## 2. What the user sees

During a window drag, each terminal's text vanishes for one frame every time its grid gains or loses a column or row. At a normal drag speed that is many times a second, which reads as a shimmer on the text. Between grid changes, for a few pixels of drag, nothing flickers. That is why it is subtle rather than constant. Agent, editor and other DOM panes never blank, so the terminal alone looks unstable.

## 3. How a terminal follows a resize today

1. **The pane's ResizeObserver fires** (`term.tsx`; `AgentShellSubblock.tsx` for the agent pane's shell drawer) and calls `TermWrap.handleResizeLive()`.
2. **The shared refit scheduler** (`term-resize-policy.ts` `createRefitScheduler`, #4408/#4658) runs the refit **synchronously** while the frame has room. That means inside the ResizeObserver callback, after this frame's animation-frame callbacks. Otherwise it runs from a later `requestAnimationFrame` callback.
3. **`refitLive` → `applyFit(dims)`:** if the grid changed, it calls `_core._renderService.clear()`, then `terminal.resize(cols, rows)`. The `clear()` is copied from xterm's `FitAddon.fit()` (added in `be64d981b`, the xterm 6 migration).
4. **Inside xterm:**
   - `RenderService.handleResize` → `WebglRenderer.handleResize`, which sets `this._canvas.width`/`height`. The canvas's backing store is reallocated and therefore empty.
   - Then `RenderService._fullRefresh()` → `refreshRows()` → `RenderDebouncer.refresh()` → `requestAnimationFrame(_innerRefresh)`.
5. **The animation-frame callback requested in step 4 runs in the next frame**, because this frame's callbacks have already run, or are running. So this frame is composited with the empty canvas.

**The second path:** `WebglRenderer` also observes its canvas (`observeDevicePixelDimensions`, a ResizeObserver on `device-pixel-content-box`). When the measured device size differs from what `handleResize` set, `_setCanvasDevicePixelDimensions` sets `width`/`height` again, emptying the canvas, and calls `_requestRedrawViewport()`, which goes through the same `requestAnimationFrame`.

## 4. Measurement

**Setup:**
- An isolated `task dev` build of `main` @ `298b82096`, Windows, 125% display scale.
- A window tab with one shell terminal beside the default panes. Another window tab held three more terminals, hidden.
- **Drag:** the window's width set 30 times through `SetWindowPos`, +20 px each, 16 ms apart.

**Probe** (injected over CDP into the dev page):
- The `HTMLCanvasElement` `width`/`height` setters were wrapped. Every real change on a canvas inside `.xterm` was recorded as an *emptying*, with a short call stack. The link-layer canvas is excluded: it only draws on link hover.
- The WebGL2 draw calls were wrapped to record each draw per canvas.
- A `requestAnimationFrame` loop installed first numbers the frames.
- An emptying counts as **blank** when the next draw on that canvas lands in a later frame: the frame in between was painted empty.
- Only terminals the browser reports visible (`checkVisibility()`) count.

**Results**, visible terminal, three drags each:

| | Before | After |
|---|---|---|
| Grid resizes from our refit | 12, 12, 12: **all blank** | 12, 13, 13: **all repainted in the same frame** |
| xterm's own device-pixel resize | 4, 3, 3: **all blank** | 3, 3, 3: **all repainted in the same frame** |
| Page frame interval p50 / p90 | 16.7–16.8 / 21.9–23.5 ms | 16.7–16.8 / 20.4–21.0 ms |

The hidden tab's three terminals were resized by xterm's paused-resize path (`_pausedResizeTask`), and their canvases were left empty. Nobody can see them during the drag, but see §7.

## 5. The fix

`frontend/app/view/term/term-paint.ts`, used by `termwrap.ts`:

- **`resizeAndPaint`** (used by `applyFit`) calls `paintRows()` right after `terminal.resize()`. `paintRows()` runs `_core._renderService._renderRows(0, rows - 1)`: the same method xterm's animation-frame callback would call a frame later, called now.
- **`TermWrap.repaintOnCanvasResize()`**, called right after `terminal.open()`, puts a ResizeObserver on the WebGL canvas (`device-pixel-content-box`). xterm registered its own observer during `open()`, so ours is called right after it in the same frame. When the canvas's size differs from what was last painted, it paints. A size `applyFit` already painted is skipped, so a refit isn't painted twice.

xterm's own animation-frame repaint still runs a frame later. It redraws the same content, so nothing visible changes.

**Tests:** `term-paint.test.ts` checks that a resize is followed by a render of every row in the same call, that an unchanged grid does nothing, and that the resize still happens without xterm's internals. `vitest run frontend/app/view/term/`: 11 files, 79 tests pass.

## 6. Architecture

**The fault line:** we treat xterm's renderer as a black box that "will repaint". A canvas renderer must paint in the frame in which it reallocated its canvas, and xterm 6 doesn't. Our code made it worse in two ways:
- **Copying `FitAddon.fit()`'s `clear()`:** it clears the render model before every resize, so nothing of the old frame survives either.
- **Driving refits from ResizeObserver and animation-frame callbacks,** the two places where a newly requested animation frame is guaranteed to be a frame late. That is the correct place to *measure* and resize; it just needs the paint to follow there too.

**Options, best first:**
1. **Paint synchronously after resize (this branch).**
   - **For:** small, measured, no visual trade-off, works for both terminal kinds (pane and agent shell drawer).
   - **Against:** depends on xterm internals (`_core._renderService`, `_renderRows`), as `applyFit` already did. An xterm upgrade must re-check them. The fix degrades safely if they're renamed: the optional calls become no-ops, and we're back to today's flicker, not a crash.
2. **Upstream:**
   - **The change:** `RenderService.handleResize` (and `WebglRenderer._setCanvasDevicePixelDimensions`) should render synchronously after reallocating the canvas, or expose a public `terminal.refresh(start, end, { sync: true })`.
   - **The case:** it is a real xterm bug that every embedder resizing from a ResizeObserver hits, with a one-frame blank for any WebGL user.
   - **After it lands:** we drop our internal calls.
3. **Don't reallocate during a drag:**
   - **How:** size the canvas to the largest grid the drag can reach, and change only the visible viewport. Or keep the grid fixed during a drag and reflow once on settle; #4399 deliberately chose live reflow instead.
   - **Verdict:** more invasive, and changes behaviour. Not needed given option 1.

## 7. Not covered here

- **A hidden tab's terminals after a resize.** xterm pauses rendering for a terminal that isn't visible and applies the resize later from `_pausedResizeTask`, which empties the canvas without painting. When that tab is shown again, the first frame may show the terminal blank until the next repaint. The flicker is invisible during the drag, but a one-frame flash on the tab switch is likely. **Not verified;** check, then apply the same synchronous paint to the "unpaused" path.
- **Cache restore at startup** (`termwrap.ts` around line 1035) resizes the grid directly, outside `applyFit`. It's a one-off at load, not part of a drag. It could use `resizeAndPaint()` too.
- **The DOM renderer** (fallback when WebGL is unavailable or `term:disablewebgl` is set) has no canvas to empty. It wasn't measured.
- **A real hand drag** (Windows' modal size loop) wasn't measured. It runs the same ResizeObserver path, so the expectation is the same.

## 8. How this was measured

The probe and drag scripts are in the author's workspace, not the repo:
- **`termprobe.mjs`:** canvas setter and WebGL draw hooks, frame numbering, a per-path report.
- **`drag.ps1`:** the scripted `SetWindowPos` drag.
- **`cdp.mjs`:** the CDP helper.
