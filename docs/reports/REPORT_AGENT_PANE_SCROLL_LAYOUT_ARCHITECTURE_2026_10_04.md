# Report: agent-pane scroll and layout — blank-space overshoot, the self-correcting "prediction", and whether the architecture needs a rethink

**Date:** 2026-10-04
**Author:** Camper (agent, `~/.agentmux/agents/camper-0622h`), at operator request
**Status:** analysis — research and recommendations only; no code changed. Findings are labelled **verified** (read in the code or measured live), **inferred** (reasoned from verified facts) or **unverified** (needs an experiment).
**Trigger:** Operator: *"it often overshoots the conversation with blank space inserted, then scrolls back to recover the blank space. what is that? looks like some prediction mechanism that corrects itself. can we get something more elegant? does it need an architecture rethink? research best practices online. … some of the files are very large, lets do a modularization and DRY analysis too."* Earlier the same session the operator also reported a faint constant "tremor" in the pane (edge glitter, cursor flipping between pointer and text).
**Related:** `SPEC_CONTENT_RESIZE_CONTRACT_2026_08_31.md` (the contract this report builds on), `REPORT_AGENT_PANE_SCROLL_PIN_FLICKER_AUDIT_2026_07_30.md`, `ANALYSIS_TOOL_CALL_SCROLL_OSCILLATION_2026_08_17.md` and its two FINDINGS follow-ups, `SPEC_AGENT_PANE_LAYOUT_REDUCER_2026_06_02.md`, `SPEC_AGENT_PANE_VIRTUALIZATION_REDESIGN.md`.

---

## 1. Summary

**It does not need a ground-up rewrite. It does need two structural changes and a measurement tool,** because the current design asks the browser to guess, then repairs the guess visibly, in two different places.

1. **Two separate mechanisms produce "blank space, then scroll back".** They look the same to a user and have different causes:
   - **A. The shrink hold (deliberate, verified).** When a row in the live turn gets shorter while the pane is pinned to the bottom, the pane keeps the old height as blank room for 350 ms and then eases the content down over 160 ms (`AgentDocumentVirtualList.tsx:413-470`). The code's own comment says "most holds ended in a visible empty gap and then an ease-down". This is the blank space the operator sees, by design.
   - **B. Estimate, measure, correct, with no compensation (verified).** Older rows are positioned from *estimated* heights until measured. A measured height that differs from its estimate moves every row below it, and nothing adjusts `scrollTop` to keep what the reader is looking at still. The estimator is a fixed 80 characters per line at 24 px, capped at 320 px, and ignores pane width.
2. **This is the tenth documented pass at one bug class.** The 2026-08-31 contract spec states the root cause: *"there is no place in this codebase where 'a node's rendered height is about to change' is a first-class event."* Six independent mechanisms each react on their own. Adding a seventh patch will repeat the pattern.
3. **Recommended direction (details in §6):** (a) first build a frame-level recorder so the next fix is measured, not argued; (b) replace the *summed-delta / no-correction* model for head rows with an **anchor-row correction** and turn native scroll anchoring off inside the list; (c) make the estimator width-aware, or avoid estimating by letting the browser skip off-screen rows with `content-visibility: auto` instead of virtualizing, after an experiment; (d) fix sub-pixel positioning under the pane's fractional zoom.
4. **The big files are mostly fine to leave, with three exceptions on the scroll path** (§7): `AgentDocumentVirtualList.tsx` (one 1,560-line function with about 25 closure variables) is the one worth splitting, because its pure parts are currently testable only through a mounted component and fake observers.

---

## 2. What the operator sees, mapped to what the code does

### 2.1 Blank space, then it scrolls back

| | Mechanism A: shrink hold | Mechanism B: estimate → measure |
|---|---|---|
| Where | Streaming buffer (the live turn), while pinned to bottom | Virtualized head (older rows), while scrolling or loading history |
| Trigger | A row gets shorter (a finished tool preview collapsing to its result: "81% of recorded shrinks" per the code comment) | A row is rendered for the first time and measures differently from its estimate |
| What happens | `min-height` is pinned to the tallest height reached, leaving blank room; after `HOLD_MS = 350` it is released and `easeOutBufferHold` animates the content down over `HOLD_RELEASE_MS = 160` | Positions come from a prefix sum of measured-or-estimated heights; the new height moves all rows below; `totalSize` changes; the browser clamps or the reader sees a shift |
| Why it exists | The browser clamps `scrollTop` synchronously on a shrink, so an eased `scrollTop` is impossible (contract spec §2). Holding the old height avoids a jump | There is no alternative in a virtualized list: unrendered rows have no height until rendered |
| Verified in | `AgentDocumentVirtualList.tsx:413-470` | `renderers.ts:36-52`, `agent-pane-layout/reducer.ts:177-220`, `AgentDocumentVirtualList.tsx:1531-1557` |

Facts about B, all **verified** by reading the code:

- **The estimator ignores width.** `estimateTextHeight` uses `TEXT_CHARS_PER_LINE = 80`, `TEXT_LINE_HEIGHT_PX = 24`, min 32, max 320 (`renderers.ts:37-52`). A narrow pane wraps more, a wide one less; code blocks, lists and tables get the same text formula or a per-kind constant.
- **There is no production feedback on estimate quality.** The estimator-miss probe is dev-only (`perf-probe.ts`, shown in the Ctrl+Shift+D HUD). There is no number for how wrong the estimates are in real use.
- **A measurement above the viewport is never used to correct `scrollTop`.** The reducer emits a `row-measured` event with a `delta`, but nothing outside the reducer and its tests consumes it. The code says native `overflow-anchor` "doesn't reliably cover it either (rows are absolutely positioned, outside anchor-candidate selection)" (`AgentDocumentVirtualList.tsx:936`). The only anchor logic is for older-history pagination (`:1324-1430`) and the pinned-to-bottom case.
- **Overscan is a fixed 5 rows** (`agent-pane-layout/types.ts`, `DEFAULT_OVERSCAN = 5`), not pixels and not scroll-velocity-aware, so a fast scroll over a stretch of tall rows can outrun what is rendered.
- **Each measurement is a separate dispatch.** The measure `ResizeObserver` loops over its entries and dispatches one `RowMeasured` per row with no `batch()` (`:1538-1556`). A measurement that moves a position triggers a full O(n) prefix-sum rebuild (`agent-pane-layout-store.ts`, the `positionsChanged` branch) and a synchronous view update. N rows measured in one callback means N rebuilds. **Inferred** from the code; not profiled.
- **Diagnostic work runs in production.** `estimateNode(...)` is evaluated on every measurement as an argument to a function that is a no-op outside dev (`:1546`, `perf-probe.ts`), and `sampleStreamingRows()` reads `offsetHeight` of every streaming row on each pin to feed a log line.

### 2.2 The tremor, the edge glitter, the cursor flip

**Root-caused later the same day (update).** An earlier revision of this section ranked the working ring, fractional `translateY` and the glide animations as suspects. The RCA below replaces that list; none of those turned out to be the cause.

**Cause: the shrink hold oscillates on its own, about every 367 ms, in any pane pinned to the bottom, including an idle one.**

1. The content `ResizeObserver` (`AgentDocumentVirtualList.tsx:1061-1091`) observes the streaming buffer. Every pass calls `holdBufferHeight()`, which writes `el.style.minHeight = el.offsetHeight + "px"` onto that same buffer and re-arms a `HOLD_MS = 350` release timer.
2. `offsetHeight` is rounded to a whole pixel, while the buffer's real height is fractional at a non-integer pane zoom (this pane runs at about 0.89). When it rounds up, the write grows the observed element by a fraction of a pixel inside its own callback. Chromium reports `ResizeObserver loop completed with undelivered notifications` and runs the pass again.
3. 350 ms later the timer clears `min-height`, the buffer shrinks back, the observer fires, the pass writes the rounded-up height again, and the timer re-arms. The cycle never ends while the pane stays pinned.

Evidence:

- **Production timing.** In the 0.59.8 host log, 97.8% of the 72,750 loop errors sit in chains spaced 350-400 ms apart; the gap histogram peaks at 360-369 ms, 24 times the background rate, with harmonics at 730 and 1,100 ms. One chain ran 46,744 consecutive cycles over 4.8 hours.
- **Version bisect.** Loop errors were tens to about 850 a day through 0.59.4, then about 90,000 to 149,000 a day on 0.59.7 and 0.59.8. The hold landed in #4178 (v0.59.5); #4217 (v0.59.6) halved `HOLD_MS` from 700 to 350.
- **Controlled repro** (headless Chrome 154, the shipped hold logic copied verbatim, nothing changing for 6 s): 16 cycles and 16 loop errors; the buffer height alternates 180.313 and 180.656 px. In 2 of 5 content geometries the scroll extent crosses a pixel-snapping boundary, and `scrollTop` flips between 2603.371 and 2602.247: **the whole conversation jumps 1.12 px and back about every 375 ms.** At zoom 1.0, with no hold, or with the hold writing `Math.floor` of the exact height, there are zero cycles. The result is the same at device scale factors 1, 1.25 and 1.5.
- **Symptoms explained.** A 1 px whole-content shift at about 2.7 Hz is the faint tremor; it re-rasterizes coloured edges at a new sub-pixel offset, which reads as glitter. The cursor flip is the expected consequence of content moving under a stationary pointer, but the repro harness did not capture it, so it is **inferred**, not shown.
- **Side effects.** The error is logged about 149,000 times a day (the bulk of a 215 MB host log), and each cycle runs the full pin pass (`syncOverflowState`, `scrollToTrueBottom`, `collapseScrolledOffTools`) in every idle pinned pane.

Fix: never let the hold write a height larger than the content, for example `Math.floor(el.getBoundingClientRect().height / zoom)`. Better still, arm the hold only on a real shrink, never from a pass triggered by its own release.

---

## 3. Why this keeps happening

The contract spec (2026-08-31) tabulates nine passes at this bug class; this report is the tenth. Its diagnosis still stands, and nothing since has changed the structure:

- Six uncoordinated mechanisms react to height change: the itemized pin effect, a viewport `ResizeObserver`, a content `ResizeObserver`, a local FLIP in `ToolOverlayLog`, a local panel auto-scroll, and throttled re-render timers in `MarkdownBlock` / `output-cap`. The list now contains five `ResizeObserver`s and the pin, hold and glide timers on top.
- Most passes produced a confident conclusion that a later pass withdrew. The behaviour lives in the interaction between observers, timers, a synchronous browser clamp and CSS transitions, and no test observes that interaction end to end.
- The hold exists *because* a shrink cannot be eased (contract §2). It trades a jump for a gap. That is a legitimate trade, but the tuning knobs (`HOLD_MS`, `HOLD_RELEASE_MS`) have been moved once already (700/220 → 350/160) and will be moved again, because the real problem is upstream: rows shrink after the reader has seen them.

---

## 4. What established practice says (research)

Researched 2026-10-04 from library docs and source, issue trackers and the CSS specs. Claims that rest on search-result summaries or that could not be confirmed are marked.

### 4.1 Virtualizers with unknown heights

- **TanStack Virtual.** The docs advise estimating the largest plausible size; `overscan` defaults to 1 ([API](https://tanstack.com/virtual/latest/docs/api/virtualizer)). `shouldAdjustScrollPositionOnItemSizeChange` corrects scroll when a row *above* the viewport measures differently from its estimate, and is **skipped while scrolling backward by default** to avoid a cascade ("items jump while scrolling up", [source comment](https://raw.githubusercontent.com/TanStack/virtual/main/packages/virtual-core/src/index.ts)). Open reports match this app's symptoms: up-scroll stutter ([#659](https://github.com/TanStack/virtual/issues/659)), flicker going back up ([#381](https://github.com/TanStack/virtual/issues/381)), reversed/chat lists drifting when estimates are poor ([#195](https://github.com/TanStack/virtual/discussions/195)). The maintainer's chat advice is to track the first visible item and adjust by its delta. `measureElement` rounds sizes with `Math.round`. The Solid adapter was not checked separately.
- **react-virtuoso.** Renders the first item as a "probe" for the default height, or takes `defaultItemHeight`; has `increaseViewportBy`, `minOverscanItemCount` and `scrollSeekConfiguration` (placeholders during fast scroll) ([props](https://virtuoso.dev/react-virtuoso/api-reference/virtuoso/)). Chat uses `firstItemIndex` + `initialTopMostItemIndex` + `followOutput`, and `data` and `firstItemIndex` must change in the same commit or the list jitters ([#1079](https://github.com/petyosi/react-virtuoso/discussions/1079), [#1511](https://github.com/petyosi/react-virtuoso/issues/1511)).
- **virtua** (has a Solid build). ResizeObserver measurement, a size cache with binary-search lookup, and its own scroll-jump compensation ([README](https://github.com/inokawa/virtua)). It has had correction bugs: absolute `scrollTop` writes raced, were changed to relative `scrollBy`, which then double-corrected when the browser clamped ([#983](https://github.com/inokawa/virtua/issues/983)). Its README warns to disable native scroll anchoring on headers and footers.
- **`@solid-primitives/virtual`:** fixed row height only, unusable here. **react-window `VariableSizeList`:** docs could not be retrieved; nothing reported.

### 4.2 Browser-native approaches

- **`content-visibility: auto` + `contain-intrinsic-size`.** Skips rendering of off-screen content while keeping it in the DOM, find-in-page and the accessibility tree; `contain-intrinsic-size: auto <len>` remembers the last rendered size ([web.dev](https://web.dev/articles/content-visibility), [MDN](https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/Properties/contain-intrinsic-size)). It still overshoots the first time a row is seen: a wrong placeholder size shifts the document by the difference (one measured case: 800 px placeholder, 1,756 px actual, [nyk.dev](https://www.nyk.dev/blog/content-visibility-measure-intrinsic-size)). The correction is persisted, not avoided.
- **A chat component that does not virtualize.** shadcn's MessageScroller uses `content-visibility: auto` plus `contain-intrinsic-size`, says this suits "hundreds to low thousands of turns", and handles bottom anchoring, streaming and prepend ([docs](https://ui.shadcn.com/docs/components/radix/message-scroller)).
- **CSS scroll anchoring** ([spec draft](https://drafts.csswg.org/css-scroll-anchoring/)). Absolutely positioned rows inside the scroller *can* be anchor candidates; a change to `top`, `height`, `margin`, `padding`, `position` or `transform` on the path from anchor to scroller suppresses the adjustment; a script-driven scroll invalidates it. A virtualizer that moves rows by `transform` therefore defeats native anchoring unpredictably. TanStack discussions and virtua both advise disabling it or avoiding conflicts. *Chromium's own implementation was not verified beyond the spec.*

### 4.3 Measuring text without the DOM

[`@chenglou/pretext`](https://github.com/chenglou/pretext) measures segments with canvas `measureText`, caches widths, and lays out with arithmetic (`prepare(text, font)` then `layout(prepared, width, lineHeight)`). Its documented limits: `white-space: normal` / `pre-wrap` only, px font sizes, avoid `system-ui`, no `font-feature-settings`, page `lang` matters, Firefox rounds. Accuracy on markdown, code blocks and inline elements **could not be verified**; fonts must be loaded first and the cache keyed by font and width. No documentation on CSS `zoom` was found. Realistic role here: exact estimates for plain paragraphs, not a general answer.

### 4.4 Chat patterns

- `flex-direction: column-reverse` opens scrolled to the bottom without JavaScript, at the cost of DOM order versus visual order (accessibility, selection) ([CSS-Tricks](https://css-tricks.com/books/greatest-css-tricks/pin-scrolling-to-bottom/)). TanStack users report needing overrides of `getScrollOffset` and `scrollToFn` to make it work ([#195](https://github.com/TanStack/virtual/discussions/195)).
- Another technique is `overflow-anchor: none` on all rows with a 1 px `overflow-anchor: auto` sentinel at the end to pin the bottom (same CSS-Tricks page). Not tested with streaming.
- How Slack or Discord do this **could not be verified**; no engineering write-ups were found.

### 4.5 Overscan and correction strategy

- Fast-scroll blanks are addressed by larger overscan or placeholders while scrolling (Virtuoso's scroll-seek). No velocity-adaptive overscan was found in the TanStack docs read.
- **Summed-delta correction** (TanStack's `scrollAdjustments += delta`) is fragile: virtua's clamp bug is the example. **Anchor-row correction** is the alternative: remember one anchor row (the first visible) and its offset from the viewport top; after each measurement batch recompute that row's position and write `scrollTop` once, skipping writes under 1 px. This is what the TanStack maintainer suggests for prepends in #195. *This is the research agent's recommendation, labelled as such, not a documented library default.*

### 4.6 Fractional zoom

No primary source on CSS `zoom: 0.89` with `translateY` was found. The closest evidence is the Mantine PR in §2.2. Options that follow from it, none verified for this app: position rows with `top` / `margin-top` rather than `transform`; snap resting offsets to device pixels; or replace CSS `zoom` with scaled font-size and spacing variables.

---

## 5. Options

Against this app's actual shape: a virtualized head plus a non-virtualized streaming buffer for the live turn, rows from one line to many screens tall, expandable tool panels, heavy streaming growth, per-pane CSS zoom.

| Option | Unknown height | Blank overshoot | Jump risk | Cost | Notes for this app |
|---|---|---|---|---|---|
| **0. Tune** (hold timings, overscan, estimator constants) | unchanged | lower | unchanged | Low | The hold has been tuned once already; treats symptoms |
| **1. Anchor-row correction + `overflow-anchor: none` in the list** | unchanged | unchanged | **Low** | Medium | Fixes B's jump; does not remove blank space on fast scroll |
| **2. Width-aware estimator** (and/or Pretext for plain text) | closer | **Lower** | Lower | Medium | Cheap, because estimates only need to be good, not exact; the estimator-miss data does not exist in production, so measure first |
| **3. No virtualization; `content-visibility: auto` + per-kind `contain-intrinsic-size`** | browser-managed, remembered | Low after first view | Low after first view | Medium | Removes B's whole class. Cost is DOM size and memory on very long transcripts. **Experiment required** (§8): the code comment claiming this is already in place is stale, see §7.3 |
| **4. Adopt virtua** | auto-measured, size cache | Low to medium | Low (has had bugs) | Medium to High | Replaces roughly the head's layout store, windowing and measure loop; the pin/hold/glide logic remains |
| **5. Contract-first: one owner for "a row's height is about to change"** | n/a | n/a | **Lower for A** | Medium to High | The 08-31 spec's step 4-6; addresses mechanism A and the six-observer sprawl |

Mechanism A (the hold) is not a virtualization problem and none of options 1-4 touch it. It is a *content* problem: rows shrink after the reader has seen them. Only option 5, or changing what shrinks (for example never replacing a tool preview with a shorter result in place), removes it.

---

## 6. Recommendation

**Do not rewrite. Do these, in this order:**

1. **Measure before touching anything (Phase 0, small).** Add a dev/diagnostic frame recorder: per `requestAnimationFrame`, log `scrollTop`, `scrollHeight`, the anchor row's `getBoundingClientRect().top`, active animations on the containers, and the hit-tested element under the last pointer position. Add a production counter for estimator miss (|measured − estimated| / measured) per kind and width. This turns every claim in §2 from inferred into measured, and ends the pattern the contract spec describes: confident analysis later withdrawn.
2. **Cheap, independent fixes (Phase 1).** None needs the architecture to change:
   - Batch measurements: one `batch` / one prefix-sum rebuild per `ResizeObserver` callback, not per row.
   - Skip diagnostic work when not probing (`estimateNode` in the measure loop, `sampleStreamingRows`).
   - Pause the working ring's rotation when the pane is not focused or under `prefers-reduced-motion`, and test whether the tremor stops (§8, test 1).
   - Round row offsets to device pixels, or move resting offsets from `transform` to `top`.
3. **Replace "no correction" with anchor-row correction for the head (Phase 2).** Set `overflow-anchor: none` inside the list so the browser and the app are not both correcting; keep one anchor row and write `scrollTop` once per measurement batch. This is the documented answer for estimate-then-correct lists and needs no new dependency.
4. **Decide the estimate problem with data (Phase 3).** If the Phase 0 counter shows large misses, make the estimator width-aware (cheap). If misses stay large for tall mixed rows, run the `content-visibility: auto` experiment (option 3) on a copy of the pane against a real 10k-row transcript, comparing DOM size, memory, first-scroll overshoot and typing latency. Adopt virtua only if option 3 fails on memory.
5. **Treat mechanism A as a separate decision.** Choose between keeping the hold with a better release (for example never release while the pointer or a recent scroll is in the live region), or removing the shrink at the source. The contract spec's steps 4-6 are the home for this.

What *not* to do: add another named dependency to the pin effect, retune `HOLD_MS` again without a recorder, or attempt an eased `scrollTop` after a shrink (contract spec §2 rules it out).

---

## 7. Modularization and DRY analysis

Scope: `frontend/app/view/agent/**` (about 79,700 source lines, 54,100 test lines) and `frontend/app/store/**` (about 23,700). Compiled by two read-only analyses on 2026-10-04; the file-size numbers and the items marked verified were re-checked by hand. Nothing here was run through a profiler.

### 7.1 Modularization

**`AgentDocumentVirtualList.tsx` (1,713 lines, about 758 of them comments).** One function with about 25 closure variables and five `ResizeObserver`s (verified count). Its responsibilities, with the shared state that couples them:

| Responsibility | Lines | Extract as | Coupling |
|---|---|---|---|
| User-scroll-intent window | 83, 186-214, 1433-1482 | `user-scroll-intent.ts` | `lastUserScrollInputAt`, `scrollbarPointerHeld`; `mark()` also cancels glides, so it takes an `onInput` callback |
| Follow-transition logging | 216-257 | `follow-log.ts` | `lastLoggedFollow`, `pendingFollowCause` |
| Pin, overflow sync, shrink diagnostic | 288-376 | `pin-controller.ts` | `pendingProgrammaticScroll` and `pinnedGeometry` are written by the pin and read by `handleScrollNow` |
| Follow decision | 1189-1322 | `follow-policy.ts`, a pure `decide({geo, stick, hadUserInput, wasProgrammatic, wasOverflowing})` | none once inputs are passed in |
| Glide and shrink hold | 378-487, 887-897 | `pin-motion.ts` | `glides`, `holdTimer`, `rowAppended` (set from JSX, read in the content observer) |
| Tail-height cache and handoff | 489-520, 798-826 | `tail-heights.ts` | `tailHeights` read in four places |
| Frontier and partition | 522-695 | `turn-frontier.ts` | `stickyFrontierId`, `frontierIndexHint`; `advanceFrontier` is already pure given its inputs |
| Layout-slice feed | 697-867 | a `useLayoutSliceFeed` hook | `pushedExpansion`, `estimatesPushed` |
| Older-history capture / restore | 1324-1430 | `older-history.ts` | `loadingOlderInFlight`; both halves resolve "head row or streaming DOM" |
| Row measuring | 504-519, 1526-1567 | `row-measure.ts` | the ÷zoom is written twice (513, 1541) |

- **Easiest wins:** `follow-policy.ts` and `user-scroll-intent.ts`. The first is pure; the second is tiny and already duplicated (§7.2). Both are testable today only through a mounted component and fake observers, so extraction adds coverage rather than only moving code.
- **Tests:** seven files (about 2,030 lines) pin behaviour (`pin`, `resize`, `handoff`, `turn-tail`, `shrink-attribution`, `collapse`, `cap-resize`). They mount the whole component through fake `ResizeObserver` and rAF, so they survive extraction if the component's props and DOM are unchanged. **Unverified:** whether any test depends on the *number or creation order* of observers; `collapse.test.tsx:26` still describes "two of these" while the component now creates five.
- **Smaller:** the two `<DocumentRow>` prop lists (`:1608-1618`, `:1693-1703`) share nine identical props.

**`agent-view.tsx` (1,495 lines, 631 comments).** One component wiring about 25 hooks; mostly orchestration already split into hooks. The scroll-relevant part is small: layout-slice registration (321-329), the scroll-to-bottom ref (848), `useScrollToNode` (1123), zoom (1150, 1263) and the props handed to `AgentDocumentView` (1332-1369). A `useAgentPaneScroll(model, block)` hook returning `{layoutView, zoomFactor, scroll, scrollToBottomRef}` would be a convenience split, not a scroll fix.

**Other large files (verdict, in priority order):**

| File | Lines | Verdict |
|---|---|---|
| `hooks/useAgentControllerStatus.ts` | 1,441 | Best split candidate: `relogin` alone is about 430 lines and `loginViaTerminal` about 180; separate login-flow modules |
| `store/agent-pane-state/reducer.ts` | 1,746 | One `update()` of 45 cases (largest about 120 lines); split into per-domain sub-reducers (init, turn, pending, stash and details, compaction) behind a thin switch |
| `store/agent-pane-state/types.ts` | 1,222 | State, a 380-line command union and a 230-line event union in one file; split `state.ts`, `commands.ts`, `events.ts` |
| `styles/_document-nodes.scss` | 3,003 | Lines 100-1861 are one `.agent-view { }` wrapper (the tool block alone is 274-1641); about 25 per-kind blocks follow. Split per node kind (`nodes/_tool.scss`, `_jekt.scss`, ...); mixins and custom properties already exist |
| `components/AgentComposerStrip.tsx` | 1,528 | About 400 lines of pure layout math (180-590) belong in `composer-strip-layout.ts` |
| `components/AgentFooter.tsx` | 1,359 | Two components in one file; move `AgentWorkingRow` (165-405) out |
| `components/AgentPicker.tsx` | 1,230 | Hooks exported from a component file (103-264) belong in `useAgentDefinitions.ts` |
| `hooks/useAgentCommands.ts` | 1,163 | An 860-line hook; split by slash-command group |
| `components/MyAgentsList.tsx` | 1,298 | A 1,000-line component; split row, sort/filter, fork/name prompt |
| `store/agent-document/reducer.ts` | 1,042 | Acceptable; the same per-domain split applies if it grows |
| `types.ts`, `useAgentStream.ts` | 1,089 / 1,034 | Pure data (split per node kind, mechanical) / one 800-line hook (medium priority) |

Priority for the scroll problem: only `AgentDocumentVirtualList.tsx` and the two modules extracted from it matter. The rest is general hygiene and should not be bundled into scroll PRs.

### 7.2 DRY

`jscpd` (`--min-lines 12 --min-tokens 70`) found **19 clones, 0.36% of lines overall and 0.11% of TypeScript**: duplication is low. It missed the scroll-path duplicates, which were found by grep.

**Scroll-related (the ones that matter):**

- **User-input window** is copied between `ToolOverlayLog.tsx` (`USER_INPUT_WINDOW_MS`, `SCROLL_KEYS`, `hasRecentUserScrollInput`, the pointer and key handlers, lines 47-52, 202-205, 251-285) and `AgentDocumentVirtualList.tsx` (83, 211-214, 1442-1482), including the `[scroll-follow]` log format. **Verified.** One home: `user-scroll-intent.ts` and `follow-log.ts`. `USER_INPUT_WINDOW_MS` is exported from the list but nothing imports it.
- **Pin-on-resize pattern** (a `ResizeObserver` whose callback pins to the bottom) is repeated in `ToolOverlayLog.tsx:225-231, 292-300`; it re-checks `isConnected`, which the list does not. The pin controller could take a "pin element" strategy.
- **Zoom normalization has three independent mechanisms:** the `zoomFactor` prop (with `rect.height / (zoom || 1)` written twice in the list and `props.zoomFactor?.() ?? 1` three times), `PeekOverlay.tsx:179-181` reading the `--agent-pane-zoom` CSS variable, and `AgentComposerStrip.tsx:1066` computing `rect.width / clientWidth`; `readZoom(block()?.meta)` is repeated in four files. One home: `zoom-factor.ts` with `cssPx(el, zoom)` and `usePaneZoom(blockMeta)`.
- **Thresholds differ without a documented reason:** near-bottom is 200 px in `anchor.ts` (`isNearBottom`), 24 px in `ToolOverlayLog` (`REATTACH_PX`), 1 px in `scroll-handoff.ts:28`.
- **Already centralized, no action:** hover/peek positioning (`peek-placement.ts`, `hover-anchor.ts`); `AgentHistoryView` reuses the same virtual list.

**General:**

- `_document-nodes.scss`: three identical "rule + label + detail" blocks (2448-2476, 2615-2643, 2674-2692), five `&::before,` rule blocks, and the `rgba(255,255,255,0.12)` fallback nine times: one `@mixin divider-row`. The pin/unpin button rule at 1587-1612 and 2914-2939 matches (specificity not checked).
- Modals: `AgentMcpModal` / `AgentSkillsModal`, `AgentNewBundleModal` / `AgentNewIdentityModal`, and `AgentCreateFromTemplateModal` share 15-25-line blocks; a shared primitive-modal component. SCSS pairs (`AgentNativeMemoryModal` / `AgentPrimitiveModal`, `_picker` / `_search`, `_btw` / `_slash`) have 14-26-line clones.
- `activity/dispatch-source.ts:82-109` with `subagent-source.ts:31-69`; in-file clones in `flows/run-provider-login.ts` (510-524, 611-630) and `agent-document/reducer.ts` (327-352, 430-437).

### 7.3 Dead or superseded code (grep-verified unless noted)

- The **no-`ResizeObserver` fallback** (`AgentDocumentVirtualList.tsx:969-983`, with unused `_len` / `_totalSize`); no list test removes the observer stub. The `typeof ResizeObserver` guards at 506, 1060 and 1531 may still be needed for jsdom mounts without a stub (not checked).
- **`blockId?` and `layoutView?` are optional but effectively required:** the only callers (`AgentDocumentView`, `AgentHistoryView`) always pass both, so the `if (props.blockId)` branches (702, 846) are dead in production. Three tests mount without `blockId` and would need updating.
- **Stale comments:** `_document.scss:131-134` says `content-visibility: auto` is applied to each `.agent-document-node-wrapper`. **Verified false:** that class no longer exists in any TSX or SCSS, and `content-visibility: auto` is declared only on attachment tiles (`_attachments.scss:199`). Also the "WHY `<Index>` NOT `<Key>`" block (`:671-678`) contradicts the `<Key>` actually rendered, TanStack references at 136, 869 and 1349 (TanStack is no longer used), and mentions of a "childList MutationObserver" (`:841`, `:1217`) that does not exist in the file.
- **Count-policy kill switch** (`:637-694`, `partitionForVirtualization`, `initialStickyFrontierId`, `STREAMING_BUFFER_SIZE`): live behind the `agent:turnscopedtail` setting, so removal is a product decision, not a cleanup.
- Exported but unused outside tests: `toggleDisclosure` (`disclosure.ts:130`), `locateIndex` (`streaming-buffer.ts:333`).

---

## 8. Experiments that would settle the open questions

Each is small and answers one unverified claim. Experiments 1-3 were written before the tremor was root-caused (§2.2) and are no longer needed for it; they remain useful only to rule the ring and fractional transforms in or out as *additional* sources.

1. **Ring test (tremor).** With the agent idle versus working, record whether the tremor and the cursor flip occur. Then add `.agent-pane-progress-bar--active::before { animation: none }` via the devtools and compare. Settles §2.2 candidate 1.
2. **Reduced-motion test.** Turn on the OS "reduce motion" setting: `reducedMotion()` disables the glides and the hold ease. If the cursor flip stops, candidate 3 is confirmed.
3. **Integer-zoom test.** Set the pane zoom to 1.0 and to 0.9: if the tremor disappears at 1.0, candidate 2 is confirmed and sub-pixel snapping is the fix.
4. **Frame recorder (§6, step 1)** during a fast scroll through older history: count frames where the anchor row's viewport offset changes by more than 1 px with no user input. This quantifies mechanism B.
5. **Estimator-miss histogram** from real transcripts by kind and pane width. Decides between option 2 and option 3.
6. **`content-visibility: auto` experiment** (option 3) against a real long transcript.

## 9. What this report did not verify

- No profiling, tracing or frame-level capture was done. Every "inferred" item in §2 and §4 is reasoning, not measurement.
- Chromium's scroll-anchoring behaviour and its handling of `zoom` with `transform` were not verified beyond the specs and one third-party fix.
- react-window, the Solid adapters, and Slack or Discord internals could not be checked.
- Whether the seven list tests depend on observer count or order was not checked.
- Some external claims come from search-result summaries rather than the primary page; the TanStack-community scroll-anchoring advice is one of them.

## Sources

- [TanStack Virtual: Virtualizer API](https://tanstack.com/virtual/latest/docs/api/virtualizer) and [virtual-core source](https://raw.githubusercontent.com/TanStack/virtual/main/packages/virtual-core/src/index.ts)
- TanStack issues: [#659](https://github.com/TanStack/virtual/issues/659), [#381](https://github.com/TanStack/virtual/issues/381), [discussion #195](https://github.com/TanStack/virtual/discussions/195)
- [react-virtuoso props](https://virtuoso.dev/react-virtuoso/api-reference/virtuoso/); [#1079](https://github.com/petyosi/react-virtuoso/discussions/1079), [#1511](https://github.com/petyosi/react-virtuoso/issues/1511)
- [virtua README](https://github.com/inokawa/virtua) and [#983](https://github.com/inokawa/virtua/issues/983)
- [web.dev: content-visibility](https://web.dev/articles/content-visibility), [MDN: contain-intrinsic-size](https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/Properties/contain-intrinsic-size), [nyk.dev: measuring intrinsic size](https://www.nyk.dev/blog/content-visibility-measure-intrinsic-size)
- [shadcn MessageScroller](https://ui.shadcn.com/docs/components/radix/message-scroller)
- [CSS Scroll Anchoring (spec draft)](https://drafts.csswg.org/css-scroll-anchoring/)
- [@chenglou/pretext](https://github.com/chenglou/pretext)
- [CSS-Tricks: pin scrolling to bottom](https://css-tricks.com/books/greatest-css-tricks/pin-scrolling-to-bottom/)
- [Mantine PR #9241 (fractional zoom, transform offsets)](https://github.com/mantinedev/mantine/pull/9241)
- In-repo: `SPEC_CONTENT_RESIZE_CONTRACT_2026_08_31.md`, `REPORT_AGENT_PANE_SCROLL_PIN_FLICKER_AUDIT_2026_07_30.md`, `ANALYSIS_TOOL_CALL_SCROLL_OSCILLATION_2026_08_17.md`
