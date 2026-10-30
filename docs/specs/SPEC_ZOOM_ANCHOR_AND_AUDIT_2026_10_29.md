# SPEC: zoom that keeps the content under the pointer in place, and an audit of the zoom system

**Status:** active. Approved by the operator 2026-10-29 with every recommendation ("approve all recommended": keyboard anchor is the pointer if over the pane else the viewport centre, optimistic local zoom in PR 1, the agent pane first). Spec task #11019; built in task #11020 by Camper-2@narko. Owner: Camper-2. Written 2026-10-29 against origin/main `0e65253fe`.
Sections 1 to 5 were read from the code, not measured; §8 records what building and the live check changed.

## 1. The request

The operator, 2026-10-29: "we need to refine our zoom system. we both want an audit and determine how much we must reasses. The one UX problem is that when zooming in we should always end up with the cursor underneath in the correct scroll position. for example, currently in the agent pane, if I zoom in, the scroll position changes and the text i intended to read out of view. we want it to work like standard zoom on photo editors or even websites."

## 2. Verdict (two lines)

**A contained fix for the zoom path, plus one small reassessment of how the factor is applied.** Anchoring is one shared mechanism (capture what is under the pointer before the change, put it back after the layout settles) that panes opt into; the agent pane is the first and the only hard one. The reassessment is that the factor is applied only after a backend round trip, so zoom lags and fast wheel ticks are lost; making the local change immediate is a prerequisite for anchoring that works, not an extra.

## 3. Audit: how zoom works today

### 3.1 The shared path (all per-pane zooms)

- **State.** One block meta key, `term:zoom`, clamped 0.5 to 2.0, rounded to 0.01, unset at 100% (`store/zoom-factor.ts`). The same key serves terminal, agent, memory, swarm, warden and every other pane whose view declares the `paneZoom` capability (`pane-tab-registry`).
- **Entry points** (all in `store/zoom.ts`): Ctrl+wheel over a pane (`AppZoomHandler` in `app.tsx` → `zoomBlockIn/Out`; xterm and CodeMirror, which swallow the wheel, use `usePaneZoomWheel` → `zoomBlockByWheel`); Ctrl +/-/0 (`keymodel` → `zoomIn/Out/Reset` on the focused block); Ctrl+Shift+wheel (`zoomAllPanesIn/Out`, a batch of independent single-pane steps); chrome zoom (`chromeZoomIn/Out`, a CSS variable `--zoomfactor` plus `sessionStorage`; scales the title bar, status bar and chrome, never a pane's content).
- **The write is asynchronous.** `stepZoom` → `setBlockZoom` → `writePaneZoom(setMeta)` → `setBlockMeta` → `RpcApi.SetMetaCommand`: a plain RPC with **no optimistic local update**. The pane's `readZoom(block().meta)` changes only when the backend's `MuxObjUpdate` echo reaches the object cache (`zoom.ts` says so itself: "MOS's local cache is NOT updated synchronously"). Consequences: a visible delay between the keypress or wheel tick and the zoom; and `stepZoom` reads the current factor from that same cache, so a second wheel tick that arrives before the first echo computes from the **stale** base (two ticks, one step).
- **No anchor information travels the path.** `zoomBlockByWheel(blockId, deltaY, baseFontSize)` is the only wheel entry that has the event, and it passes none of `clientX/clientY` on. `AppZoomHandler` has the event too. The keyboard paths have no pointer at all.
- **Nothing preserves scroll across a zoom** in the shared path: no code near `zoom.ts` touches `scrollTop`, `scrollBy` or an anchor.

### 3.2 The agent pane (the reported case)

`agent-view.tsx:1132` reads `zoomFactor = createMemo(() => readZoom(block()?.meta))` and applies it as **CSS `zoom` on `.agent-view-zoomed`** (line 1231), a wrapper inside the pane (the Shell drawer sits outside it on purpose). `zoomFactor` also goes to `AgentDocumentVirtualList` (line 1334) and `useScrollMemo`.

What happens on a zoom change, from the code:

1. The echo arrives; `zoomFactor()` changes; the wrapper's CSS `zoom` changes. Under CSS `zoom` the wrapper's **layout width becomes `container / zoom`**, so the transcript's text **reflows**: its line breaks, and with them every row's height in layout px, change non-proportionally.
2. The scroller's `scrollTop`, `clientHeight` and `offsetTop` are in **unzoomed layout px** (the list's own comments: CDP-confirmed at zoom 0.5 and 2); `getBoundingClientRect` is in zoomed px. So the same `scrollTop` number now points at different content, because all the rows above it changed height. That is the operator's "the text I meant to read is out of view".
3. The list records the new factor in the layout slice (`dispatchLayoutIfRegistered(blockId, { type: "ZoomChanged", zoom })`, AgentDocumentVirtualList line 872) with the stated invariant "INV-2: stored, never relayouts": it adopts the factor for its ÷zoom conversions and does **not** re-place the scroll. Row heights are then re-measured by ResizeObservers (÷zoom) over the following frames, each one moving the layout under a fixed `scrollTop`.
4. **Pinned to the bottom** is safe: `follow-scroll.ts:121` ignores Ctrl/Cmd+wheel (it never reaches the follow controller as an escape), and the follow controller's invariant I1 says no geometry change or anchoring adjustment can leave FOLLOWING; a resize while following re-pins. So a pinned pane stays pinned, by design.

So the cause the task suspected is **confirmed with two corrections**: (a) it is not only the async write; the reflow of the zoomed wrapper plus fixed `scrollTop` is the cause of the jump, and the async write adds lag and lost ticks; (b) the pane already has the machinery to put a scroll position back after layout changes (row anchors, below), it is just not used for zoom.

**Anchoring pieces that already exist in the pane** (reusable): `keep-on-screen.ts` (`captureOnScreen` records the rows on screen and the viewport top's offset past each; `restoreOnScreen` finds the first still there and puts the scroll back at the same offset from its new start; no DOM reads, positions come from the layout slice), `anchor.ts` (`captureTopmostAnchor`, `restoreScrollFromAnchor`), `scroll-memo.ts` (`captureScrollMemo` reads the first row that starts on screen from the DOM, unzoomed), and the list's `scrollToNode(nodeId, offsetPx)` (also stops following).

### 3.3 Every other zoom path

"Anchors" means some code keeps the content under the pointer across the change.

| Pane / path | Mechanism | Where the factor lives | Write | What scrolls | Anchors on the pointer today? |
|---|---|---|---|---|---|
| **Agent pane** | CSS `zoom` on `.agent-view-zoomed` (`agent-view.tsx:1231`); text reflows | `term:zoom` meta | async (RPC, then echo) | the transcript scroller (`AgentDocumentVirtualList`, virtualized rows) | **No.** The list converts coordinates to unzoomed px (`one-way-recorder.ts` does so only to *sample* frames for diagnostics, not to hold a position), and `keep-on-screen`/`scroll-memo` exist but are not called for zoom |
| **Terminal** | xterm font size = base x zoom, rounded (`stepZoom` skips factors that round to the same size) | `term:zoom` meta | async | the xterm buffer (xterm's own viewport) | No. xterm refits and keeps its own scroll rule |
| **Editor** | CSS `zoom` on `.editor-view`, plus a live zoom atom (`editor-view.tsx:204`, `model.zoomAtom()`) | `term:zoom` meta + atom | async meta, live atom | CodeMirror's scroller | No |
| **Files, Kanban, Swarm, Memory, Help, Tower, Toolchain, Settings, Warden** | CSS `zoom` on the view's root | `term:zoom` meta (Help migrated from `help:zoom`) | async | each view's own list or board; Files and Tower virtualize | No |
| **Sysinfo** | `--sysinfo-zoom` CSS variable scales fonts | `term:zoom` meta | async | a virtual list | No |
| **Image viewer** | transform: translate + scale on the image | local `ZoomState` (`image-zoom.ts`) | synchronous | pan, not a scroll | **Yes.** `zoomAt(state, factor, cursor, box)` keeps the image point under the cursor fixed (`x: cursor.x - ratio * (cursor.x - state.x)`), fed by the wheel and click handlers that do pass `clientX/clientY` (`fromCentre`). The only pointer-anchored zoom in the app |
| **PDF viewer** | pdf.js `scale` | pdf.js's viewer | synchronous | pdf.js pages | Not in this code. pdf.js has an `origin` option on its scale update that anchors a point (to be checked in the build) |
| **Drone** | pane zoom skipped; the graph has its own pan/zoom in flow coordinates | graph state | synchronous | graph viewport | Its own screen-to-flow maths; not part of `term:zoom` |
| **Browser pane** | native page zoom in CEF; not driven by `term:zoom` | the page | the page | the page | Whatever Chromium does; out of scope |
| **Chrome zoom** | CSS variable `--zoomfactor` (title bar, status bar, chrome) | `chromeZoomAtom` + `sessionStorage` | synchronous | none (never scales pane content) | Not applicable |

**Everything that goes through `term:zoom` shares the same four facts:** it is clamped 0.5 to 2.0, written asynchronously with no local update, applied by CSS `zoom` or a font size, and has no anchor. **Ctrl+wheel handlers that exist:** `AppZoomHandler` and `AppAllPanesZoomHandler` in `app.tsx`, and `usePaneZoomWheel` for xterm and CodeMirror (`term.tsx:235`, `editor-view.tsx:206`). **None of them passes the pointer position on**, except the image viewer, which has its own handler. Keyboard: `keybindings/defaults.ts:117-120` (Ctrl +/-/0, Ctrl+Shift+= and wheel for all panes), dispatched by `keymodel` to the focused block.

*(Audit method: read from the source and cross-checked at the agent pane, terminal, editor and image viewer; the other CSS-zoom panes were classified from their use of `term:zoom` and the shared stepper. An automated read of the code claimed the agent pane already anchors through `one-way-recorder.ts`; reading it shows that file only samples row positions for diagnostics, so that claim is wrong and corrected above.)*

## 4. Recommendation

### 4.1 One shared mechanism: `ZoomAnchor`

In `store/zoom.ts`, plus a small hook panes opt into. No pane learns about the wheel.

1. **Every zoom entry takes an optional anchor** `{ clientX, clientY }`: `zoomBlockByWheel(id, deltaY, baseFontSize, anchor)`, `AppZoomHandler` and `usePaneZoomWheel` pass the event's position; the keyboard entries pass none and the pane substitutes (§5). `stepAllPanes` passes each pane its own centre (the pointer is over one pane at most).
2. **Capture before the change, restore after the layout.** `stepZoom` calls `captureZoomAnchor(blockId, anchor)` *before* it writes; the pane's registered capture function records what is under the point and returns a token. A pane opts in with `useZoomAnchor({ blockId, zoom, adapter })`, where `adapter` is a small object the pane supplies: `getAnchor(point) -> token` and `scrollToAnchor(token)`. **The hook owns the timing and nothing pane-specific** (capture before the write, restore after the layout and again over the next few frames while measurements land, stop when the user scrolls or two passes agree, skip when the pane is following or hidden, drop the token); **the pane owns its own scroll rules** through the adapter. The agent list's adapter is the DOM-point anchor with the row anchor as fallback (`captureOnScreen` for `getAnchor`, `scrollToNode(nodeId, offset)` for `scrollToAnchor`), so the list keeps its INV-2 and live-arrival rules. A live-arrival adjustment in the same frame wins: the adapter reports it and the hook re-captures. (Agreed with AgentX, the agent pane's owner.)
3. **A DOM-point anchor, not a ratio.** The natural photo-editor maths (`scroll' = (scroll + y) * z1/z0 - y`) is wrong for a reflowing pane. The anchor is the **content under the pointer**: the nearest `[data-node-id]` row, the text position inside it (`document.caretPositionFromPoint` / `caretRangeFromPoint`, a `Range`), and the pointer's `y` inside the scroller. Restore: find the same position, read its new rect, and scroll by the difference from the saved `y` (`scrollTop += (newTop - y) / zoom`, in unzoomed px as the list wants). If the row was unmounted by virtualization, fall back to the **row anchor** (`captureOnScreen`/`restoreOnScreen`, the row under the pointer and the offset in it) through `scrollToNode(nodeId, offset)`.
4. **Settle, then stop.** Restore runs after layout in the same frame as the factor change, and again after each of the next few frames while the row measurements land (the ResizeObserver heights), until two consecutive passes agree or the **user scrolls** (a wheel or key without Ctrl, a scrollbar drag), which cancels it. A cap of about 10 frames bounds it.
5. **Optimistic local zoom (the reassessment).** `store/zoom.ts` keeps a per-block **local zoom** that is set immediately by `setBlockZoom` and read by `stepZoom` and the panes (`paneZoom(blockId)` = the local value while a write is in flight, else `readZoom(meta)`); the echo reconciles it. It makes the zoom apply in the same task as the gesture (no round trip), fixes the lost ticks, and gives the anchor a synchronous point: capture, set, restore. Without it the anchor still works (it keys on the factor changing), but the zoom still lags and drops ticks.

### 4.2 What changes, what stays

- **Changes:** `store/zoom.ts` (anchor argument, local zoom, `captureZoomAnchor`), `app.tsx` `AppZoomHandler` and `usePaneZoomWheel` (pass the pointer), `agent-view.tsx` (read the pane zoom accessor, use the hook), the agent list (register capture/restore on the existing row-anchor machinery), keymodel (pointer or centre for the keyboard).
- **Stays:** `term:zoom` as the persisted value, the clamp, the indicator, the all-panes stepper, chrome zoom, the follow controller (it already treats zoom correctly), `INV-2` (the layout slice still stores the factor and never relayouts; the restore is a scroll write after, not a relayout).

### 4.3 Sizing, per pane

| Family | Work | Estimate |
|---|---|---|
| **Shared core** | `store/zoom.ts` (anchor arg, local zoom, `captureZoomAnchor`), `useZoomAnchor`, `AppZoomHandler`/`usePaneZoomWheel`/keymodel pass the point, tests | 1 to 1.5 days |
| **Agent pane** (the reported case) | capture/restore on the row-anchor machinery plus the DOM-point anchor, settle loop, follow no-op, live-arrival yield, recordings | 1.5 to 2 days; the hard one |
| **Terminal** | anchor on the buffer line under the pointer, restore with `scrollToLine` after the refit | 0.5 to 1 day |
| **Editor** | anchor on `posAtCoords`, restore with `EditorView.scrollIntoView` after the zoom | 0.5 day |
| **CSS-zoom list panes** (Files, Kanban, Swarm, Memory, Help, Tower, Toolchain, Settings, Warden) | the generic DOM-point anchor on the pane's nearest scroller; no per-pane logic. Files and Tower virtualize: they need a row anchor | 1 day for the batch (small each; Files and Tower the larger two) |
| **Sysinfo** | its `--sysinfo-zoom` variable change on a virtual list: the generic hook plus a row anchor | 0.5 day |
| **PDF viewer** | pass the pointer as pdf.js's `origin` | 0.5 day, if the option works as documented |
| **Image viewer, Drone, Browser pane, Chrome zoom** | none (already anchored, own scheme, native, or not content) | 0 |

Roughly **5 to 7 days in all, 2.5 to 3.5 for the shared core plus the agent pane**, which is what the operator asked about; the rest is mechanical once the core exists. This is a **contained fix**: one mechanism, no pane rewritten, the persisted state and the follow machinery unchanged. The one reassessment is the local zoom (§4.1 item 5): it changes where the pane reads its factor from, so it touches every pane's `readZoom(meta)` call (about a dozen call sites, mechanical).

## 5. Edge cases

- **Pinned to the bottom (agent pane):** do nothing; it stays pinned (§3.2). Capture is skipped when following (the hook checks `follow.following()` at capture time). While the pane follows, `OneWayFlow` (`scroll/one-way-flow.ts`) is the only `scrollTop` writer and snaps the latest message to the bottom after a zoom; the zoom anchor owns the write only for a detached reader (via `scrollToNode`). **If the pane starts following during the settle loop, the restore is abandoned.** (Agreed with Loap-3, whose bottom-tight spec #11293 relies on this.)
- **Clamped zoom (already 0.5 or 2.0):** `stepZoom` returns the same factor; no capture, no write, no scroll movement. The indicator still shows the limit.
- **Rapid wheel steps:** the anchor is captured at the **first step of a gesture** and kept until about 200 ms of no zoom input; later steps reuse the token while the pointer has not moved more than a few pixels, and re-anchor if it has. With the local zoom each step applies at once, so consecutive restores do not fight a late echo.
- **Keyboard zoom (Ctrl +/-):** the pointer if it is over the pane (the last known pointer position, which the window already tracks for Ctrl+wheel), else the **vertical centre of the viewport** (the line the reader's eye is most likely on); never the top edge, which would shove the reading line off screen when zooming in. For a pane with a text cursor (editor, terminal) the cursor line wins.
- **All-panes zoom (Ctrl+Shift+wheel):** each pane anchors on its own viewport centre; the pointer anchors only the pane under it.
- **Zoom while the pane is hidden or has no size:** no anchor (nothing to read); the factor applies as today.
- **A new message arrives during the gesture:** the live feed's own anchoring (`keep-on-screen`) and this one both move `scrollTop`; the zoom restore yields to a live-arrival adjustment in the same frame and re-captures from the row anchor, never fighting it.
- **Touch pinch / trackpad pinch:** browsers deliver a trackpad pinch as Ctrl+wheel with fractional `deltaY`; the same path handles it. A touch pinch has no handler today and is out of scope.

## 6. Plan

1. **Confirm live** (first step of the build, in the build's own `task dev`): record the jump in the agent pane at 100% to 110% and 100% to 150%, to see whether the reflow or the lag dominates, and the same for two other panes. Also confirm that `caretPositionFromPoint` / `caretRangeFromPoint` return the right text position under CSS `zoom` (Chromium's coordinates for zoomed content have been inconsistent). If they are off, the **row anchor is the primary** and the DOM point is used only for the offset inside the row (AgentX).
2. **PR 1 (everything in one, nothing split out; approved by AgentX for the agent pane):** local zoom + the anchor argument + `useZoomAnchor`, with the agent pane on it (pointer and keyboard), tests (anchor maths on a fake scroller, clamp no-op, gesture re-use, follow no-op, **a live-arrival yield during the settle loop leaves the pane re-anchored once, not twice**, **the local zoom reconciles to the echo when the write was clamped or rejected**), and recordings.
3. **PR 2 onward:** one PR per other family in the sizing table, smallest first.

## 7. Decisions for the operator

1. **Keyboard anchor:** pointer if over the pane, else the viewport centre (recommended), versus always the top visible line.
2. **Optimistic local zoom in PR 1** (recommended: it removes the lag and the lost ticks and is what makes anchoring exact), versus anchoring only and leaving the echo lag.
3. **Panes to include in the first pass:** the agent pane only (recommended; it is the reported one and the hardest), then the others in the order of the sizing table.

## 8. Changes made while building (2026-10-30)

PR 1 is the shared core and the agent pane (§6 step 2); the other families in §4.3 follow as their own PRs.

1. **Local zoom** is `pane-zoom-local.ts`: `setBlockZoom` holds the level it wrote, `paneZoom(blockId, meta)` returns it in place of the meta, and the stepper starts from it (two wheel ticks before the echo are two steps). It is let go 1.5 s after the write settles, at once if the write is refused, and an older write settling never lets go of a newer level. Only the agent pane reads `paneZoom` so far; the other panes keep reading the meta until their PR, and the stepper's base is the held level for all of them.
2. **The restore measures, it does not replay.** Each pass looks at where the held glyph is now and scrolls by the difference (÷ zoom), so a live arrival or a late row measurement that already moved the text leaves nothing for the pass to undo, and the "re-anchored once, not twice" case needs no separate rule. A pass that finds the text in place counts as agreeing; two agreeing passes, a scroll by the reader, the pane starting to follow, or 10 frames end it.
3. **The engine's answer is checked.** `caretPositionFromPoint` returns the nearest text when the point is over none (a gap, a spacer); in the lab that was text a screen away, which sent a first version to the top of the transcript. The text is used only when it is in a row and within a line of the point; otherwise the row under the point (or the nearest mounted one) is the anchor, held by its start in the layout slice.
4. **Checked live** (a `task dev` build on its own home, a scripted fake agent with a 160-paragraph reply, Ctrl+wheel through CDP at one point): `caretPositionFromPoint` and the glyph rectangles are right under CSS `zoom` (the row and the glyph's y stayed within about 4 px of the pointer from 100% to 144% and back, in 3 steps and in 6 ticks 20 ms apart, in and out); keyboard Ctrl +/- with the pointer outside the pane held the pane's vertical centre; a pane that follows stayed pinned at the bottom both ways; at 200% a step moved nothing. With the scroll left where it was (the old behaviour) the same zoom left the pointer over text from line 1057 of the transcript instead of line 1154.
5. **The file-size baseline** for `AgentDocumentVirtualList.tsx` is raised by 2 lines (the import and the one-line call); everything else lives in `zoom-anchor-adapter.ts`.
