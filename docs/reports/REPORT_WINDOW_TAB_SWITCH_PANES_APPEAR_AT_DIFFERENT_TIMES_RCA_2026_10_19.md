# RCA: on a window-tab switch, panes of different kinds appear at slightly different times

**Date:** 2026-10-19
**Status:** RCA from the code, **not yet measured**. §3 says what the code guarantees, §4 which gaps it produces and how big they should be, §6 the fixes, §7 how to measure before building any of them.
**Author:** korp
**Trigger:** Operator, 2026-10-19: *"when switching window tabs, the misc panes load at different times, it is quick and subtle, but there."*
**Related:** `docs/analysis/ANALYSIS_WINDOW_TAB_SWITCH_PAINT_2026_09_30.md` (the DOM side: one frame for a warm switch), `docs/specs/SPEC_AGENT_PANE_NO_CATCH_UP_MOTION_2026_10_15.md` §3.7 (the agent cover), `docs/analysis/ANALYSIS_BROWSER_PANE_RESIZE_ARCHITECTURE_2026_10_06.md` (the rect batcher).
Code citations are against `main` @ `f0b2df57c`.

---

## 1. Summary

A warm window-tab switch is one frame **for everything that lives in the page's DOM** (09-30 analysis, §7). Two kinds of pane do not follow that frame, and each has its own reason:

1. **Browser panes** are not in the DOM. Each is a separate native window that sits above it. The DOM swap hides the leaving tab's placeholders and shows the new tab's, but the native windows only move when the page tells the host to, over IPC: an effect, then a task hop, then `browser_panes_set_rects`, then a UI-thread task on the host, then `SetWindowPos`. A pane coming in is moved from 0×0 to its real size, so its page also has to lay out and paint again at the new size before it has content. A pane going out is collapsed the same way. So browser panes appear and disappear **after** the rest of the tab, by the length of that chain.
2. **Agent panes** pause their rendering while hidden, by design, and draw what they held back on the show. When the transcript changed while the tab was hidden, the transcript is **covered** until its size has held still for 2 frames, at most 350 ms (`SHOW_COVER_MAX_MS`). The composer and the pane's header show in the swap frame; the transcript shows later, and how much later depends on how much that agent did while hidden.

Everything else (terminals, sysinfo charts, editors, swarm, files, settings) has nothing to catch up on and shows in the swap frame. That is why the staggering is visible at all: the panes that are on time make the late ones stand out. Two agent panes in one tab can also differ from each other, because each covers for as long as its own backlog takes to lay out.

A third, smaller source is the first visit to a tab (§4.4), and a fourth is a tab so large that the swap frame is itself late (§4.5); neither staggers panes, but they read as "loading".

## 2. What was and wasn't checked

- **Read:** the tab container logic (`workspace.tsx`, `window-tab-visibility.ts`), the one visibility signal (`pane-tab-visibility.ts`), every consumer of it, the agent show path (`settle-on-show.ts`, `agent-dormancy.tsx`, `usePaneReveal.ts`, `AgentDocumentVirtualList.tsx`), the browser path (`use-pane-rect-sync.ts`, `pane-rect-batcher.ts`, host `crates/cef/src/browser_panes/bounds.rs`), the terminal and sysinfo show paths, and the earlier analyses.
- **Not done:** no trace of a real switch with these pane types in it. The mechanisms in §3 are read from the code and are certain; the **sizes** of the gaps in §4 are estimates from the pieces other analyses measured, marked as such. §7 is the measurement that settles them.

## 3. How a warm switch reaches each pane

### 3.1 The DOM swap (the same for every pane)

`workspace.tsx` renders every window tab, stacked. `displayTabId` flips (optimistically for a warm destination, `resolveDisplayedTabId`); `tabContainerVisibility` then shows the new tab and, for two frames, hides the old one with `content-visibility: hidden` before its `visibility: hidden` applies. `WindowTabDisplayedProvider` hands each pane `tid === displayTabId()`.

`usePaneTabVisibility(blockId)` folds that with pane-stack dormancy into `"active" | "dormant" | "windowHidden"`. It is **the** signal; the consumers are:

| Consumer | What it does when it flips to `active` |
|---|---|
| `block.tsx` | `onActivate()` on the view model; focus if the pane is selected |
| `agent-view.tsx` | `renderDormant` goes false: the agent resumes drawing |
| `scroll/settle-on-show.ts` | lands at the bottom, may cover the transcript (§4.2) |
| `use-pane-rect-sync.ts` | sends the browser pane its real rect (§4.1) |
| `blockstats.tsx` | asks for the remote figures at once |
| `AgentHistoryView`, `PaneTabStrip`, `AgentPicker`, `usePaneReveal` | smaller per-pane reveals |

Terminal, sysinfo, editor, swarm and files read nothing from it. They are plain DOM inside a tab that stayed laid out, so nothing is pending when they are shown.

### 3.2 Why the browser pane cannot be in that frame

`use-pane-rect-sync.ts` `syncPosition()`: when `visibility() !== "active"` the rect it sends is `{0,0,0,0}`, otherwise the placeholder's real rect. A `createEffect` on `visibility()` calls it. The path from there:

1. Effect runs in the swap's own task: `rects.set(blockId, rect)` records the newest rect.
2. `pane-rect-batcher.ts`: the flush runs as a **new task** (`MessageChannel`), and only if no batch is in flight. If one is out, the new rect waits for its answer.
3. `browser_panes_set_rects` crosses to the host. `bounds.rs` `set_rects` posts one UI-thread task per window if none is waiting, and the IPC call waits for it.
4. UI thread, `apply_windows`: one `DeferWindowPos` per parent moves every pane's wrapper. The wrapper's `WM_SIZE` sizes CEF's window inside it.
5. A pane coming in goes from 0×0 to its size. Its renderer must lay out and paint at the new size, and the compositor must present that frame. Only then does the pane have content.

None of 1–5 is on the DOM's frame schedule. The DOM compositor presents the new tab at its next vsync; the native windows move when the host's UI thread gets to its task and when the page behind each one has produced a frame. Panes of the **leaving** tab are collapsed by the same batch, so for a short time an old pane can still be drawn over the new tab (it is above the DOM) or a new pane can be missing from it.

### 3.3 Why the agent pane is staggered on purpose

`agent-dormancy.tsx`: a hidden agent pane keeps receiving the stream and updating its store, but **stops drawing** (markdown, tool bodies) so a tab nobody sees doesn't take the main thread from the one being typed into. On the show it draws everything it held back, and the design in `SPEC_AGENT_PANE_NO_CATCH_UP_MOTION` §3.7 hides that: `useSettleOnShow` covers `.agent-document` (`visibility: hidden; opacity: 0`, still laid out) when the document's nodes changed while hidden, and lifts the cover after `SHOW_COVER_QUIET_FRAMES = 2` frames without a resize, or `SHOW_COVER_MAX_MS = 350` ms after the first frame following the show. Chrome stays visible. A pane with no new content shows at once; a pane scrolled up is never covered.

That cover is the right trade for the text moving (it fixed the "fast scroll"), but it is a per-pane timer that starts at the show. The lab report measured about 330 ms of drawing before the first painted frame in a packaged build for a heavy backlog.

## 4. The gaps this produces

Sizes marked *(est.)* are not measured here.

### 4.1 Browser pane: appears after the tab, content last

- Chain in §3.2 steps 1 to 4: a few ms to tens of ms on an idle host; longer while another batch is in flight (the 200 ms safety-net interval only sends on change, so this is mostly a drag or a page-load storm) or the UI thread is busy *(est.: 1 to 4 frames)*.
- Then the 0×0 to real-size repaint of the page *(est.: 1 to 3 more frames, more for a heavy page)*.
- A native pane has no transition and no cover, so the reader sees an empty rectangle (the pane's background) and then the page.

### 4.2 Agent pane: composer first, transcript later

- Covered from the show until two quiet frames or 350 ms. For a pane with a large backlog, up to 350 ms *(measured by the 10-15 lab for the draw, not for this cover)*; for one with a small backlog, 2 to 4 frames *(est.)*.
- Several agent panes in a tab each run their own cover, so they lift at different times. Panes that were idle while hidden are not covered at all, so the same tab shows some transcripts at once and others a moment later.

### 4.3 Everything else: on time

Which is why the two above are noticed. This is the baseline the 09-30 work achieved (content in one frame at about 30 to 60 ms).

### 4.4 First visit to a tab ("cold")

A tab not yet shown goes through the reveal gate: it waits 80 ms without long tasks (`tab-reveal.ts`) and fades for 120 ms. Tabs a window loads with are marked shown once their content has *settled* (`markLoadedTabsShownWhenSettled`), but settling only counts views that opt in with `trackPaneContent` (agent, picker, sysinfo, swarm, files). **A tab holding a browser pane, terminal or editor never reports settled, so its first reveal stays gated** (09-30 §9). That makes the first pass through the tabs after a launch slower and more staged than later ones.

### 4.5 A heavy tab's swap frame is itself late

The 09-30 analysis measured 70 to 90 ms to swap into a 6,800 to 11,000 element tab (restyling the inherited `visibility`, first paint). Everything in that tab appears in that one late frame, so it reads as a hitch, not as staggering. Listed so it isn't mistaken for either of the above.

### 4.6 Smaller

- `startHiddenTabWatcher` (`pane-overlay.ts`) re-measures every overlay on the marker flip and sends the overlay clip to the host, so the cut-outs over browser panes also change after the swap.
- `block.tsx` `onActivate` hooks and the focus move run in the same effect as the show; none of them paints.

## 5. Why this was not seen before

The 09-24 and 09-30 work measured the **DOM**: pill, tab content, and the late frame from `transition: all`. The trace probe watched the page's own frames, and the native browser windows are not in them. The agent cover shipped on 10-15, after the 09-30 numbers, and it was judged on text motion, not on the time at which the transcript appears relative to its neighbours.

## 6. Possible fixes

Ordered by how directly each removes a gap. Each is independent. None is built.

### 6.1 Agent panes

**A. Pre-warm on intent (recommended).** A tab switch is announced before it happens: `pointerdown` on the pill comes 30 to 60 ms ahead of the swap, and hover comes earlier. On `pointerenter`/`pointerdown` of a window-tab pill, lift the render dormancy of that tab's followed agent panes **while the tab is still hidden** so they draw their backlog off screen, with the cover still on. By the show most of the work is done and the cover lifts after its two quiet frames. If the pointer leaves without a switch, they go dormant again. Cost: a burst of rendering for panes the user may not open, bounded to what the pill is over; the single-signal design in `pane-tab-visibility.ts` already routes it (add a "pre-warm" input to the dormancy memo).
**B. Render the backlog at idle while hidden.** Time-sliced in `requestIdleCallback` chunks, with a budget per frame, instead of all on the show. Smaller bursts, but it brings back some of the hidden-pane main-thread cost the dormancy exists to remove (`ANALYSIS_AGENT_PANE_TYPING_UNDER_LOAD_2026_09_22.md`); it needs a cap on how many panes and how much per second.
**C. Cover the whole pane, not only the transcript.** Chrome and transcript then appear together. It makes the agent pane consistent with itself but not with its neighbours, and it adds up to 350 ms of empty pane. Not recommended on its own.
**D. Share one cover across the tab.** Hold the swap itself until every covered agent pane has lifted. Consistent, but makes every switch into a tab with a busy agent take up to 350 ms. Not recommended.

### 6.2 Browser panes

**E. Send the show with the swap, not after it.** Today the rect leaves in an effect, then a task hop. Flush the batch synchronously inside the effect that flips `displayTabId` (skip the `MessageChannel` hop for a visibility change) and let the host apply it without waiting for a 200 ms-clamped path. Removes one hop, not the host round trip.
**F. Pre-send on intent.** Same trigger as A: on `pointerdown` of a pill, send the destination tab's browser panes their real rects **hidden-but-sized** (see G) so that on the swap they only need to be made visible. If the click is cancelled, send them back.
**G. Hide, don't collapse (recommended with F).** Instead of resizing to 0×0 and back, keep each pane at its real size and toggle `ShowWindow` or the wrapper's visibility. A pane coming in then has a current frame to present, with no relayout from zero. Needs care on Windows (CEF's window must keep `WasHidden(false)` rendering or the first frame after the show is stale) and equivalents on macOS/Linux overlays; `input_owner` already keys off the "collapsed" rect, so the keyboard hand-off needs the same signal.
**H. Hold the swap for the ack (only if E to G aren't enough).** Delay the DOM swap until `browser_panes_set_rects` returns, but only when the leaving or arriving tab has a browser pane. Everything then changes together, at the cost of the IPC time on exactly those switches. It is the only fix that makes the tab truly atomic; it should be the last resort because it costs latency.

### 6.3 Cold visits

**I. Count browser, terminal and editor panes as settled once mounted.** They don't need to report data loading to be shown: give the settled check a "mounted and has a first frame" path for views that never opt in, so a tab holding them doesn't take the gated first reveal. (A browser pane's first page load stays its own business, with its own indicator.)

### 6.4 Measure-only

**J. Put per-pane arrival into the trace.** §7.

## 7. How to confirm, before building any of it

One dev instance of `task dev` from this checkout (not the operator's installed one), one window, four window tabs; each tab has: 2 streaming agent panes (one idle while hidden, one working), a terminal, a sysinfo chart, a browser pane on a page that animates (a clock or `about:blank` with a ticking title is not enough; use a canvas that redraws).

1. Reuse the 09-30 probe and trace recipe (§6 of that analysis): `pointerdown` on the pill, per-frame screenshots from `Tracing` with `disabled-by-default-devtools.screenshot`, diffed per **pane rectangle** instead of per region.
2. **Browser panes are not in the page's screenshots.** Capture them from the OS: `PrintWindow` or `Graphics.Capture` of the whole window at 60 fps, or a screen recorder, so the native windows are in the frames. This is the part the old analysis could not see.
3. Per switch, record the frame in which each pane first shows its final content: header, transcript, browser content, terminal, chart. Report the spread (latest minus earliest pane) per switch, for warm and cold, with a streaming agent and with an idle one.
4. Pass: spread within 1 frame (16.7 ms) on a warm switch, no frame in which an old browser pane still covers the new tab.
5. Run each fix (A, E/F/G, I) against the same fixture; ship behind a setting like `window:keepinactivetabslaidout` was, so the operator can A/B it live.

Expected from this RCA, to be confirmed: the spread is dominated by browser panes (a few frames) and covered agent transcripts (2 frames to 350 ms), and the other panes show in the first frame.

## 8. Recommendation

1. Measure first (§7). It's an hour with a fixture and settles which of 4.1 and 4.2 the operator is seeing.
2. If it's agent panes, do **A** (pre-warm on intent). If it's browser panes, do **E** and **G**, then **F** if a gap is left; keep **H** in reserve.
3. **I** is independent and cheap; do it with whichever of the above goes first.

Agent: korp@narko
