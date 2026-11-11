# Report: interaction quirks audit, ranked from the recorded long frames and a static sweep

**Date:** 2026-11-10
**Status:** analysis. Log reading and code reading only; no lab was run. Proposed measurements are listed in §6 and need the operator's OK. The strip guard of §3.1 and the uPlot bind of §3.5 ship in the same change as this report (task #47405).
**Author:** Loap-3@narko (task #47377)
**Data:** the operator's instance, host logs of 0.60.20 to 0.60.22, 2026-11-05 to 2026-11-11 (14,053 `[perf] long-frame` lines; 10,180 of them 0.60.21, which is the build analysed row by row). Source read at `origin/main` f837e615b.

## 1. Summary

- **One bug is worth more than everything else together: a 12-minute loop on 2026-11-10 (16:53 to 17:06 UTC).** It wrote 3,134 long frames and 500 s of frame time, 39% of all long-frame time in the week, in a single episode. One busy agent pane's scroller flipped between zero size and full size 5,529 times, and the resize callbacks of the attachment strips in its history re-ran every frame. The window was deactivated for 10.5 of those minutes (16:54:50 to 17:05:22) and the loop stopped about 25 s after the window was activated again. The same flip shows up in smaller bursts on another pane on three days. Nothing in the log says what changes the scroller's box; §6 proposes the measurement that would.
- **Splitting AgentY's ResizeObserver row:** of the 4,107 `ResizeObserverCallback` frames, 3,115 are that one loop (the attachment strip). Taking the loop out, the next ranked rows are the stream flush (2,106 frames), the frames with no script at all (2,450), the scroll follower's observer (491) and the shared layout-container observer (101).
- **The second largest row is unexplained:** 2,450 frames (338 s, median 83 ms) have no script and no style or layout time either (median render 2 ms). The main thread was busy outside both. The log cannot say what with.
- **Clicks and focus:** the `#document.onfocusin` handlers cost a median of 159 ms (272 ms in 0.60.22), `#document.onclick` 111 ms, a press in a system-info chart 247 ms (maximum 1.1 s). Three causes found in code: a host IPC on every focus move, a layout-tree reducer on every pane focus, and uPlot's document-level mouseup.
- **Static sweep:** the `html[...] *` shape exists only where already known (1 in `app.scss`, 4 in `floating-pane-workspace.scss`, filed as #47376). The 11 `:has(` selectors are all local and cheap. Details in §4.
- Counts are floors: the long-frame logger writes at most 5 lines a second, and 2,089 frames were reported as "not logged" in the week.

## 2. Pass 1: what the long frames say

Method: parse every `[fe] [perf] long-frame` line (duration, blocking, scripts, render, style+layout, the three slowest scripts with invoker, file and character position, and the "+N more" count). Group by the slowest script of each frame. Resolve the positions of 0.60.21 against the bundle that build served (the page's cached copy; positions are UTF-16 offsets, so Python indexes must be taken over UTF-16 code units, or they drift by one per astral character, 85 characters by the 2.2 MB mark).

### 2.1 0.60.21, every row (10,180 frames, 1,273 s)

The loop (2026-11-10 16:53:20 to 17:06:00) is shown apart. "Forced" is the layout forced inside the script.

| # | Rows | Frames | Time | Median / max | Forced | What it is |
|---|------|-------:|-----:|-------------:|-------:|------------|
| L | RO callback, attachment strip, in the loop | 3,134 | 499.6 s | 149 / 1,516 ms | 317 s | `AttachmentStrip.tsx:63`, see §3.1 |
| 1 | no script (style, layout, paint only) | 2,450 | 338.2 s | 83 / 1,647 ms | 0 | not attributable, §3.2 |
| 2 | rAF `runFrame` | 2,106 | 182.5 s | 73 / 899 ms | 16.9 s | `stream-scheduler.ts:runFrame`, the per-frame stream flush |
| 3 | RO callback, scroll follower | 491 | 36.9 s | 65 / 305 ms | 2.0 s | `scroll/one-way-flow.ts:123` `onResize` |
| 4 | `WebSocket.onmessage` | 474 | 35.7 s | 62 / 455 ms | 3.6 s | host event dispatch, handler not named |
| 5 | `#document.onmouseup` (system-info chunk) | 83 | 27.6 s | 247 / 1,103 ms | 11.9 s | uPlot, §3.5 |
| 6 | rAF, scroll handler | 373 | 24.8 s | 62 / 191 ms | 0.3 s | `AgentDocumentVirtualList.tsx:1105` `handleScrollNow` |
| 7 | `import().then` (rasterise) | 189 | 22.2 s | 104 / 431 ms | 0.4 s | drag ghost `toPng`, §3.6 |
| 8 | RO callback, layout container | 101 | 21.3 s | 115 / 1,260 ms | 3.8 s | `layoutModelHooks.ts:120` `useOnResize` |
| 9 | `#document.onfocusin` | 69 | 16.9 s | 159 / 1,019 ms | 4.7 s | §3.4 |
| 10 | `#document.onclick` | 59 | 7.3 s | 111 / 321 ms | 3.8 s | delegated handler, not named |
| 11 | `setTimeout` (event bus deferral) | 79 | 6.0 s | 59 / 351 ms | 0.5 s | `iSe` in the event module |
| 12 | `#document.onpointermove` | 47 | 4.9 s | 87 / 240 ms | 1.4 s | pane splitter drag, `TileLayout.win32.tsx:60` |
| 13 | `BUTTON.onclick` | 17 | 3.9 s | 229 / 460 ms | 1.2 s | an icon-button click |
| 14 | `DIV.onscroll` | 41 | 3.5 s | 80 / 180 ms | 0.2 s | `follow.scrolled()` |
| 15 | `DOMWindow.ondrop` | 10 | 2.5 s | 206 / 816 ms | 1.1 s | pragmatic-dnd drop |
| | the rest (about 60 rows) | | under 2.5 s each | | | |

Delegated Solid events (`onclick`, `onfocusin`, `onpointermove`) all show as the framework's one dispatcher function, so the log cannot name the handler that ran. That is the largest remaining blind spot (§5).

### 2.2 0.60.22 (745 frames, 93 s; the operator's instance from the upgrade to 04:24 UTC on 11-11)

Too short and too quiet to rank against 0.60.21 (about 10 hours, mostly overnight). Same families: no script 170 frames, `onfocusin` 45 frames (median 272 ms), `onmessage` 101, `toPng` 86, stream flush 114, `onpointermove` 63, scroll rAF 40, RO 27. No loop. Only four zero-size flips were logged after the upgrade.

### 2.3 What already-merged work removes

Release 0.60.21 predates all of them; 0.60.22 contains them. The log cannot measure the effect (different use), so this is by mechanism only.

| PR | Merged (UTC) | Removes from the table above |
|----|--------------|------------------------------|
| #6890 hidden keep-alive pane skips layout while the window resizes | 11-10 04:46 | part of rows 3 and 8 (the same resize frames) |
| #6891 My Agents grid works out its columns from its width during a resize | 11-10 04:57 | part of row 8 |
| #6916 the text-drag guard marks on the first move | 11-10 16:41 | part of the click and focus cost: no whole-document restyle on a press in an input |
| #6926 browser panes' rects reach the host over a CEF process message | 11-10 17:23 | the `browser_panes_set_rects` IPC cost in §2.4 |
| #6921 reopened pane keeps tool rows closed; a quiet window gets its GC | 11-10 11:31 | the memory spike and the resulting GC pauses on open; no row above |

Not removed by any of them: the loop, the stream flush, the no-script frames, the follower's observer on its own, the uPlot mouseup, the ghost rasterise, the focus path.

### 2.4 Slow IPC calls (the `[perf] ipc` lines, calls over 16 ms, 0.60.20 to 0.60.22)

| Command | Slow calls | Time | Median / max |
|---------|-----------:|-----:|-------------:|
| `input_select` | 1,174 | 138 s | 74 / 1,387 ms |
| `vault_op` | 493 | 128 s | 31 / 4,556 ms |
| `browser_panes_set_rects` | 892 | 103 s | 56 / 2,486 ms |
| `browser_panes_set_overlay_clip` | 870 | 43 s | 33 / 350 ms |
| `capture_window_viewport` | 159 | 42 s | 204 / 1,244 ms |
| `update_wco` | 170 | 26 s | 42 / 2,613 ms |
| `list_window_instances` | 202 | 25 s | 46 / 1,255 ms |

On 11-10 the host recorded 600 input selects and 443 of them were slow IPCs: three in four focus requests take over 16 ms to come back. The calls are not awaited, so the page is not blocked by the wait, but the host thread that serves them is the one that serves the keyboard.

## 3. Ranked findings

Ranked by frame time first, then by how directly a person feels it.

### 3.1 The zero-size / sized loop on a busy pane (and the attachment strips it drags along)

- **Does:** the person is away (or switching); one busy pane's transcript scroller is reported as `ch=0` then `ch=2074` again, several times a second, for 12 minutes. Every flip is a `scroll-follow` line: `following→suspended cause=zero-size` (5,529 of them in the loop) and `suspended→following cause=sized`. The scroll height varies each cycle (12,945 to 13,781 px), so the content is being laid out again each time. The attachment strips in that transcript each get a resize notification per cycle.
- **Cost:** 3,134 frames, 499.6 s, median 149 ms, 63% of the callback time is forced layout. Median 2 strips per frame, up to 8. At 4 frames a second the main thread is saturated for the whole episode: the same minutes show long-task lines of 150 to 200 ms back to back, the `[tool-result] … can't read the result back (line 3436587 …)` line 95 times in 3 minutes.
- **Where else:** another pane flipped the same way on 11-08 (262 flips in one minute, 23:06), 11-09 (325, 04:08 to 04:15) and 11-10 (241, 02:20). Counts of `cause=zero-size` per version: 0.60.15 9, 0.60.17 3, 0.60.18 6, 0.60.20 17, 0.60.21 6,384, 0.60.22 4.
- **Code:** the state machine that logs it is `frontend/app/scroll/follow-controller.ts:243-250` (`BecameZeroSize` and `BecameSized` pin or suspend with no hysteresis). The zero case is `frontend/app/view/agent/scroll/one-way-flow.ts:258-279` (`ch <= 0` returns early and drops the baseline). The strips' callback is `frontend/app/view/agent/attachments/AttachmentStrip.tsx:63` (`setWidth(e[0]?.contentRect.width ?? 600)`), which re-runs `trayLayout` (`tray-layout.ts`) and the `<For>` of tiles on every width change. What toggles the scroller's box is NOT established. Candidates, none confirmed: the keep-alive slot's `content-visibility: hidden` (`tab/pane-leaf-members.tsx:61,91`, set while the window is resizing or the pane lags behind), the show-cover, and a hide/show done by the window being deactivated.
- **Owner:** AgentX (agent pane, scroll follower) with Camper (keep-alive slots, layout).
- **Fix shape:** (1) find the toggle with the loop detector of §5 (needs one more occurrence); (2) make the follower ignore a zero-size report that reverts within a frame, and stop re-pinning on each `sized`; (3) in `AttachmentStrip`, skip a callback whose width equals the last one or is 0, and fold the update into one animation frame.
- **Is it fixed in 0.60.22?** Unknown. No loop was logged after the upgrade, in about ten hours of mostly idle time.

### 3.2 Frames with no script and no layout

- **Does:** anything. 2,450 frames, 338 s, median 83 ms, 90th percentile 247 ms, maximum 1.6 s; 94% are single frames (2,114 of 2,256 episodes), spread across every hour of the day; 5 in a row at most.
- **Cost:** 27% of the frame time of 0.60.21 outside the loop. Median render 2 ms and style+layout 1 ms, so the time is before the render step and outside any script: garbage collection, the browser process (a synchronous message), image decode, or a long paint that the entry does not count.
- **Code:** none can be named. In the 0.60.22 data it is 23% of frames.
- **Owner:** AgentY to route (it may be the CEF seat, Korp).
- **Fix shape:** a measurement first (§6). Cheap addition to the log line: the wait before render (`renderStart - startTime`), the page's visibility state, and a GC marker (`performance.measureUserAgentSpecificMemory` is too heavy; `PerformanceObserver('gc')` is not offered, so use the host's own memory log lines to match times).

### 3.3 The stream flush

- **Does:** an agent pane receives output while the person works.
- **Cost:** 2,106 frames, 182.5 s, median 73 ms, 9% forced layout (script time, not layout). It is the top row after the loop and the no-script frames. The scheduler already limits panes per frame while the person types (`stream-scheduler.ts` `runFrame`), so these frames are one pane's flush.
- **Code:** `frontend/app/view/agent/stream-scheduler.ts:runFrame` runs each pane's `flush` (its document write: new and updated nodes, `FlushObserved`).
- **Owner:** AgentX (agent pane).
- **Fix shape:** log the pane, the rows written and the milliseconds per flush (§5); then bound the flush by work (rows or bytes), not by pane count.

### 3.4 Focus changes

- **Does:** click into a pane or into its input box.
- **Cost:** `#document.onfocusin` 69 frames in 0.60.21 (median 159 ms, 4.7 s forced), 45 in 0.60.22 (median 272 ms, forced 13%, so mostly script); `#document.onclick` 59 frames, median 111 ms. #6916 took out the whole-document restyle of a press in an input; the script part is not touched.
- **Code, from reading:** every pane frame has `onFocusIn={props.blockModel?.onFocusCapture}` (`block/blockframe.tsx:1388`), which is `handleChildFocus` (`block/block.tsx:296-324`): `nodeModel.focusNode()` when the pane is not focused, then **always** `getApi().reclaimWindowFocus(windowLabel)`, which is the numbered `input_select` IPC (`host/cef-host-commands.ts:160-185`, slow in three of four calls, §2.4). `PaneChrome.tsx:224-225` also calls `focusNode()` on `click` and `focusin`. `focusNode` (`frontend/layout/lib/layoutFocus.ts:189-205`) returns early only when the node is already the focused one; otherwise `model.treeReducer` runs a layout action. Which of the two takes the 159 to 272 ms cannot be told from the log (delegated events).
- **Owner:** AgentX (pane focus), Camper (layout reducer).
- **Fix shape:** call `reclaimWindowFocus` only when the focus came from outside the main page's own focus owner (the host knows; send it only after a browser pane was last), and time `focusNode`'s reducer separately.

### 3.5 A press in a system-info chart

- **Does:** click in a system-info plot.
- **Cost:** 83 frames, 27.6 s, median 247 ms, maximum 1.1 s, 43% forced layout. Rare, but each one is a quarter second.
- **Code:** uPlot binds `mouseup` on the document at mousedown (`node_modules/uplot/dist/uPlot.esm.js:5818`, `onMouse(mouseup, doc, mouseUp, false)`), and its `mouseUp` reads the plot's rect (`cacheMouse`), which forces layout of everything dirty. `sysinfo-plot.tsx:308-312` turns the drag off (`drag: { x: false, y: false }`) but leaves the cursor binds, so the mousedown and mouseup handlers still run.
- **Owner:** no seat named for system info; AgentY to route.
- **Fix shape:** set `cursor.bind.mousedown` and `cursor.bind.mouseup` to `() => null` for these plots (uPlot treats null as "do not bind"), or read the rect from the existing size signal.

### 3.6 The drag ghost

- **Does:** rest the pointer on any pane for a quarter of a second.
- **Cost:** 189 frames, 22.2 s, median 104 ms, maximum 431 ms (0.60.22: 86 frames, 9.1 s). The code's own comment puts `toPng` of a pane at about 170 ms.
- **Code:** `frontend/layout/lib/TileLayout.core.tsx` (`generatePreviewImage`, fed by `previewIntent.enter`, `TileLayout.core.tsx:694`) after `DRAG_PREVIEW_HOVER_MS = 250` (`drag-preview-size.ts:139-148`).
- **Owner:** Camper (layout, tabs).
- **Fix shape:** rasterise on `press` only (the intent object already has `press`), or in an idle callback at a smaller canvas size; a hover is not a drag.

### 3.7 Resize observers on the scroll follower and the layout container

- **Cost:** the follower's observer (row 3) 491 frames, 36.9 s, median 65 ms; the layout container (row 8) 101 frames, 21.3 s, median 115 ms, maximum 1.26 s. The follower observes the scroller, its containers and **every row** (`one-way-flow.ts:136-145`); one resize batch makes `onResize` re-run for all of them in one callback. The layout container's callback is `layoutModel.onContainerResize`, the whole tree's recompute.
- **Owner:** AgentX (follower), Camper (layout).
- **Fix shape:** the follower's callback should do its row work only for entries whose border box really changed (compare `borderBoxSize` to the kept one); the layout callback should skip an unchanged container size.

### 3.8 The rest of the table

`WebSocket.onmessage` (row 4) and the event-bus `setTimeout` (row 11): the handler that ran is not named. The scroll rAF (row 6) is the scroll handler coalesced to one a frame (`AgentDocumentVirtualList.tsx:1099-1110`); 62 ms median is the work it does, and a log of the rows it touched would say whether it scales with the transcript. The splitter drag (row 12) is `layoutModel.onResizeMove` every 10 ms (`TileLayout.win32.tsx:27-33`), which re-lays out the tree: expected, but 87 ms means the drag runs below 12 frames a second.

## 4. Pass 2: the static sweep

I did this sweep by grep and reading myself. Two subagents started on it failed on network errors, so no findings here come from them.

| Shape | Count | Worth fixing |
|-------|------:|--------------|
| (a) a root marker with a universal descendant | 5 in styles: `app.scss:22` `html[data-text-drag] *` (now marked on the first move, #6916) and `floating-pane-workspace.scss:55,58,61,64` `html[data-floater-edge=…] *` | The four floater rules (filed as #47376). The marker changes only when the pointer crosses a floating window's edge, so the cost is per edge crossing, but each one restyles the whole document. |
| (a) root marker plus descendants of a component | `mixins.scss:507` `:root[data-motion-reduce~=…] &` in the `when-motion-reduced` mixin | No: it changes only when the motion setting changes. |
| (b) JS writes to `documentElement` and `body` | `app.tsx:112-145` (window opacity, theme, polarity: on a settings change), `store/zoom.ts:338` (`--zoomfactor`: on a zoom step), `init/dpr.ts:15` (`--dpr`: on a pixel-ratio change), `init/pool.ts:34,38`, `motion/motion.ts:73-74` (tier and reduce: on a setting), `store/global.ts:197` (platform class: once), `view/editor/editor-zoom-resize.ts:54-59` (body `cursor` and `user-select` during a zoom drag), `workspace/floating-pane-workspace.tsx:230-231` (the floater edge), `element/ui/text-drag.ts` (the text drag marker) | Only the last three run on input; `user-select: none` on `body` (inherited) restyles the document once at drag start and once at the end, the same shape as `data-text-drag`. Low. |
| (c) `:has(` selectors | 11 (the brief said 16), all anchored on a small element with a sibling or child condition: `PaneTabStrip.scss:248`, `StatusBar.scss:101-119` (4), `_document.scss:107-109` (3), `_pending-footer.scss:87`, `_recent-sessions.scss:147`, `browser-view.scss:174`, `swarm-seats.scss:7` | None. The widest is the tab strip's, and a tab opening or closing is rare. |
| (d) IPC on every focus / press / key | 1 confirmed: `block.tsx:318` `reclaimWindowFocus` (§3.4). Drag-time calls in `floating-pane-workspace.tsx` are throttled or coalesced. | `block.tsx:318`. |
| (e) layout reads in input, scroll, resize and RO handlers | 50 `new ResizeObserver` sites in source, 79 reads marked `perf:allow-layout-read` | The sites in §3.1 and §3.7. I did not read all 50; the log names three as costly. |
| (f) permanent document / window listeners for move, scroll and wheel | `app.tsx:256,296` (`wheel`, non-passive, but it returns before any DOM query unless Ctrl or Cmd is held), `follow-scroll.ts:328`, `focusManager.ts:284`, `floating-pane-workspace.tsx:328,1084,1085`, `useWindowDrag.win32.ts:231`, `pane-overlay-auto.ts:177` (`scroll`, capture, passive) | Not seen in the log (no `onwheel` row above 1.2 s total). Low. |
| (g) app-root signals read by many panes | reader counts only: `activeTabId()` 40 reads in 27 files, `activeBlockId()` 33 in 6, `windowFocused` 25 in 5, `focusedBlockId` 26 in 10 | Not analysed for change frequency; the focus path (§3.4) is the one the log points at. |

Also found, not in the brief's list: three body-wide `MutationObserver`s (`notification/use-toast-placement.ts:93`, `platform/pane-overlay-auto.ts:163` on Windows, `platform/pane-overlay.ts:84`). The second adds and removes tracking for every inserted or removed node in the page, which a streaming, windowed transcript does all day. No mutation callback appears in any long frame, so it is not measured as a cost; it belongs in the first trace of §6.

## 5. What the next audit's log lines must carry (Pass 3)

The interaction line (#47378, PR #6968) gives event, input delay, processing and presentation, window, pane block id and view, and target role or marker. For the next audit to need no source maps and no bundle, the long-frame line (`frontend/perf/long-frames.ts`) and its neighbours must also carry:

1. **The source position resolved at log time** to a file and function (`source-map-resolver.ts` exists for crash stacks), for the slowest script only and cached by position. This removes the whole manual step of §2.
2. **The delegated handler's name for Solid events.** The framework dispatcher hides it; the slow-interaction line's target and event type, joined by time, is the substitute until then.
3. **A resize-loop detector:** per observer site, more than 20 callbacks in 5 s writes one line with the observed element described the way the interaction line does (role, marker, pane), its size before and after, and the callback count. It would have named the loop of §3.1 in its first second.
4. **For no-script frames:** the wait before render, the visibility state, and whether the window was active.
5. **One line per stream flush over 16 ms:** pane id, rows written, milliseconds.
6. **Logger headroom:** 2,089 frames were dropped by the 5-lines-a-second cap. Raise it when the frames are over 100 ms, or write a per-minute count by row so the totals are exact.
7. **The window label** on each long-frame line (the logs do not say which window a frame came from).

## 6. Proposed measurements (not run; each needs the operator's OK)

1. **Find what toggles the scroller's box in the loop (§3.1).** A scripted run on the operator's build is not needed: a task dev build of a pane with a long history and attachment rows, window deactivated, `performance.now()` log of the scroller's `clientHeight` per ResizeObserver callback plus a trace of style invalidation, 5 minutes. Decision it informs: where to put the hysteresis.
2. **A 60 s Chromium trace of ordinary use** (focus changes, a streaming pane, hover on panes) on a task dev build, with the `devtools.timeline`, `v8.gc` and `disabled-by-default-devtools.timeline.invalidationTracking` categories. Decision: what the no-script frames are (§3.2), whether the body-wide mutation observers cost anything, and the true split of the focus handler (§3.4).
3. **Before and after of the strip fix** once it exists, on the same history.

## 7. What I could not establish

- Which window and pane each long frame belongs to (the line has neither).
- The cause of the loop's size toggle, and whether 0.60.22 or 0.60.23 still has it.
- Anything about the 0.60.22 and 0.60.23 builds beyond the first ten hours.
- Rows 4, 10 and 11 beyond the dispatcher they run in.

Agent: Loap-3@narko
