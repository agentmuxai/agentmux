# REPORT — A tab's panes shift up by half the window height (tile-layout scrolled)

**Status:** implemented — fixes 1 and 2 of §5 ship with this report (`overflow: clip` on `.tile-layout`, `preventScroll` in the agent pane's `giveFocus()`). Fix 3 (settle-aware refocus) is folded into the tab-switch analysis rather than patched here.
**Date:** 2026-09-24
**Author:** agentx
**Observed on:** local 0.57.0 portable (`07a2e91e1`), Windows, "Window 1 - Tab 2" (4 panes). Reported as a recurring anomaly ("we have seen this before").
**Related:** #3519 (`SPEC_PANE_SELECT_AUTOFOCUS_2026_09_22.md`), `frontend/layout/lib/tilelayout.scss`, `frontend/layout/lib/layoutModel.ts` (`overlayTransform`), `frontend/app/store/focusManager.ts`, `frontend/app/view/agent/agent-model.ts` (`giveFocus`).

---

## 1. Symptom

All 4 panes of Tab 2 were drawn about half the window height too high. The window
controls (close/minimize) and the top half of each conversation sat off-screen
above the window, with an empty band at the bottom. The rest of the app was
normal. The user couldn't scroll it back.

## 2. What was measured (live, read-only, over DevTools on the 0.57.0 host)

| Element | overflow | scrollTop | clientHeight | scrollHeight |
|---|---|---|---|---|
| `html`, `body`, `#main`, visual viewport | — | **0** | 1176 | — |
| **`div.tile-layout.animate`** | **hidden** | **560.8** | **1120** | **11,120** |

- **The only unexpected scroll offset in the page is on `.tile-layout`**, the
  container that holds every pane of the tab. It's `overflow: hidden`, so the
  user can't scroll it. Something scrolled it *programmatically*.
- **560.8 is half of 1120**, the "about half the window height" the user saw.
- The focused element was an agent composer, `textarea.agent-input`
  (block `7f6080b0…`).
- **It persisted.** Re-measured at 13:35 UTC, after the user had switched tabs
  three times (`workspace.SetActiveTab` at 13:25:45, 13:26:04, 13:26:12), Tab 2's
  `.tile-layout` still read `scrollTop 560.8`. The user's own screenshot at
  13:34:32 shows the same. (An intermediate probe that read `0` used
  `querySelector('.tile-layout.animate')`, which matched **Tab 1's** layout. Both
  tabs' layouts are live in the DOM. That reading was wrong.)
- Per-pane geometry in Tab 2 at 13:35. The left column is split: top pane
  `b7d418da` and bottom pane `29a37a12` (Swarm). The 560 px shift put:
  - `b7d418da` at y −493…32, entirely above the window;
  - the three agent panes at y −493…592, with their headers and window
    controls off-screen;
  - the Swarm pane at y 67…592. It was the *bottom-left* pane, so its header
    is the only one visible, which is why the screenshot looks "almost right"
    on the left.

  The whole lower half of the tab is empty.

## 3. Why an `overflow: hidden` box can be scrolled at all

`.tile-layout { overflow: hidden }` (`tilelayout.scss:8`) clips visually, but it
is still a **scroll container**. Script can scroll it: `scrollTop`,
`scrollIntoView()`, and `focus()`, which scrolls every scrollable ancestor to
reveal the focused element. It only needs somewhere to scroll *to*, and the
layout provides that:

- `.tile-layout` has three absolutely positioned children: `display-container`
  (the panes), and `placeholder-container` + `overlay-container`, parked
  off-screen with a `transform` of **at least 100,000 px** when no drag is
  active (`layoutModel.ts`, `overlayTransform`: `Math.max(100000, top + 2 *
  height)`). The large floor is deliberate. It fixed the 2026-07-11
  "dead source tab" incident, where a zero-height rect parked the placeholder
  on top of the tab.
- **Per-pane drag previews are parked 10,000 px down inside the display
  container:** `.tile-preview-container { position: absolute; top: 10000px }`
  (`tilelayout.scss:118`), one per pane, rendered off-screen so the drag ghost
  can be measured. The `Placeholder` is likewise rendered at `top: 10000px`
  (`TileLayout.core.tsx`). Tab 2's `scrollHeight` of **11,120** (10,000 +
  1,120) is exactly this: DevTools showed a small parked copy of every pane at
  y ≈ 9,500–10,600.
- Absolutely positioned and transformed descendants count toward scrollable
  overflow, so **every tab's `.tile-layout` has a scroll range of about 10,000
  px, or about 100,000 px while the overlay is parked by transform** (Tab 1 read
  101,121). Any of them is enough for the mechanism below.

## 4. What scrolled it

### 4.1 The call

Since **#3519** (merged 2026-09-23, "Auto-focus a pane's input on selection"),
selecting a pane focuses its input programmatically:

- `layoutModel` tree actions **InsertNode, FocusNode, MagnifyNodeToggle** (and
  the other selection cases) → `focusManager.requestNodeFocus()`
- → `refocusNode()` **synchronously**, right after the tree state commits
- → `giveBlockFocus(blockId)` → the pane's `giveFocus()` → for agents,
  `agent-model.ts`: `ta.focus()` **without `{ preventScroll: true }`**
  (the editor's `cm.focus()` is the same shape).
- #3519 also added `focusManager.claimFocusOnMount(blockId, () => vm.giveFocus())`
  in `AgentFooter`'s `onMount`, which focuses when a pane **mounts**.

- #3519 also added a refocus to **tab switching**: `tab-actions.ts`
  `setActiveTab()` calls `focusManager.refocusNode()` two animation frames
  after the `SetActiveTab` RPC returns. Its own comment says this is so "the
  destination tab's panes have had a chance to mount". Nothing checks that
  the destination tab's layout has *settled*. This is the path that fired in
  this occurrence (§4.5).

### 4.2 Why the offset is exactly half

Chromium's focus-scroll uses *center-if-needed*:
- target fully visible: no scroll;
- target partly visible: scroll to the nearest edge;
- target **entirely** outside the scroller's box: scroll to **center** it.

Centering in a 1,120 px box sets `scrollTop = targetCenter − 560`. A measured
560.8 means the target was entirely out of view, centered at content y ≈ 1,121:
**just past the bottom edge of the tab**.

That's where an agent composer can be while a pane's layout is **in flux**:
while it's being inserted, resized or magnified, or while a tab is being
switched into (§4.5). The paths above fire during that window:
- `requestNodeFocus()` runs synchronously after the tree commits, before the
  new layout has painted;
- `claimFocusOnMount` runs when the new pane mounts;
- and `.tile-layout.animate` transitions pane positions for 150 ms
  (`--animation-time-s: 0.15s`).

The composer sits at the bottom of its pane, so while the pane is still at its
old or pre-animation geometry (or its content hasn't laid out yet), the
composer can be at or past the tab's bottom edge. Focus scrolls `.tile-layout`
to center it, the animation then finishes with the panes in their correct
places, **and the 560 px offset stays**.

### 4.3 Why it sticks, even across tab switches

- Nothing ever resets `.tile-layout.scrollTop`. There is no scroll handler or
  reset anywhere in `frontend/layout`, and the user can't scroll an
  `overflow: hidden` box.
- **Switching tabs does not clear it.** Tabs are kept alive: both tabs'
  `.tile-layout` elements measured with real, non-zero rects. Inactive tabs are
  hidden without `display: none`, so the scroll box survives and keeps its
  offset. Verified: three tab switches, still 560.8 nine minutes later.
- Untested workarounds: a window reload (the frontend is rebuilt from
  scratch), or from DevTools
  `document.querySelectorAll('.tile-layout').forEach(t => t.scrollTop = 0)`.

### 4.4 "We have seen this before"

The mechanism predates #3519. Any focus or `scrollIntoView` that reaches an
off-screen element inside `.tile-layout` produces it. Other programmatic focus
calls without `preventScroll` exist in the agent pane (search bar,
`InAppLoginPanel`, `MyAgentsList`, `useAgentDropAttach`, `AgentFooter`'s
completion and undo paths) and in `blockframe.tsx` (the rename input). #3519
made it **routine**, though. Every insert, magnify or selection now focuses an
input, often mid-animation, so the conditions for the offset arise far more
often.

### 4.5 The trigger in this occurrence: switching back into Tab 2

The user's account was: Tab 2 was fine; they switched to Tab 1 and collapsed
and restored panes there; they switched back to Tab 2 and it was offset. The
host log (`[fe]` lines, UTC) matches it exactly:

| Time | Event |
|---|---|
| 13:22:04 | `SetActiveTab` → Tab 1 (`e9420f73`) |
| 13:24:00, 13:24:02 | Tab 1's layout `2743c05b` updated twice (the collapse/restore) |
| **13:25:45.09** | **`SetActiveTab` → Tab 2 (`f52315e4`)**; reveal gate lifts at +125 ms |
| 13:25:46.85 | Tab 2's right agent pane `7f6080b` logs its conversation scroller at `clientHeight=1090` |
| 13:25:47 – 13:25:56 | eight `ResizeObserver loop completed with undelivered notifications` errors |
| 13:28:34 | first DevTools capture of the offset (`scrollTop 560.8`) |

The causal chain:

1. **The switch into Tab 2** runs `setActiveTab()`. Two frames after the RPC
   returns, #3519's `focusManager.refocusNode()` focuses Tab 2's focused pane:
   the right agent pane `7f6080b0`, whose composer is still the active
   element today. The call goes through `giveFocus()` → `ta.focus()`
   *without* `preventScroll`.
2. **Tab 2's layout had not settled.** It had been
   `content-visibility: hidden` for 3½ minutes while its agents streamed.
   `content-visibility: hidden → visible` does not lay the subtree out at
   once (see `workspace.tsx`'s own comments). The workspace forces the
   top-level layout with a `getBoundingClientRect()`, but the panes' own
   follow-up work runs over the next frames: ResizeObserver callbacks, the
   virtual list re-measuring, the composer area resizing. The evidence:
   - The right pane's conversation scroller measured **1090 px, 1.7 s after
     the switch**. Its settled value is **1123** at 13:18, 13:28 and 13:34.
     The composer/footer region was still changing size then.
   - The ResizeObserver-loop errors over the next ten seconds show Tab 2
     still relaying out.
3. **At the moment of focus the composer was out of view.** The settled
   layout doesn't explain the offset:
   - The composer sits at content y 1,095–1,117. That's inside the
     1,120 px box, 3 px above the bottom edge. Focusing it there scrolls
     nothing.
   - The actual offset requires a target *entirely* below 1,120, centered at
     about 1,121 (§4.2).
   - So when focus ran, the composer/caret was drawn just past the pane's
     bottom edge. The pane ends at 1,119.5, flush with the tab's bottom.
     This is the transient layout from step 2.
   - With only 3 px of margin, a small transient shift is enough.
4. **Chromium centered it** by scrolling `.tile-layout` by 560.8 px (§3
   explains why that box can scroll at all).
5. **It stuck** (§4.3). Once scrolled, the composer sits mid-window, fully
   "visible", so later focus calls are no-ops. That includes the refocus from
   the user's next switch into Tab 2 at 13:26:12. Nothing resets the offset.

**The Tab 1 collapse/restore was not the cause.** Minimize state is per tab
(`2743c05b` is Tab 1's layout; Tab 2's is `034ce00f`), and nothing in
`layoutMinimize.ts` touches another tab. What mattered on the Tab 2 side:
- it was hidden for minutes while its agents kept streaming, so there was
  real catch-up work on reveal;
- its focused pane is an agent whose composer sits right at the tab's bottom
  edge;
- the tab-switch refocus fires on a fixed two-frame delay, not after the
  destination settles.

It's a race, which is why most switches today (10:57 through 13:05) didn't
produce it.

Not recoverable after the fact: the exact size of the transient shift, and
which of the catch-up steps in step 2 produced it. Instrumenting
`refocusNode()` to log the target's rect and the scroller's `scrollTop`
before and after would settle it on the next occurrence.

## 5. Fixes, in order of strength

**Shipped with this report:** 1 (`tilelayout.scss`) and 2 for the agent pane
(`agent-model.ts`; #3519's on-mount claim goes through the same
`giveFocus()`). Terminal and editor focus go through xterm and Monaco, whose
internal `focus()` calls this change can't reach. With 1 in place they can no
longer scroll `.tile-layout` either. Both guards are pinned by
`frontend/layout/tests/tileLayoutFocusScroll.test.ts`. 4 is unnecessary once 1
is in: there's nothing left to scroll.

1. **Make `.tile-layout` not a scroll container: `overflow: clip` instead of
   `overflow: hidden`** (`tilelayout.scss:8`). `clip` clips exactly the same
   but, per CSS Overflow 3, the element is not a scroll container and
   programmatic scrolling is impossible. That closes the whole class,
   regardless of which focus or scroll call fires or how containers are parked.
   One line. Check before merging: anything that relied on `.tile-layout` being
   a scroll container (nothing in `frontend/layout` reads or sets its
   `scrollTop` today), and sticky-positioned descendants.
2. **`focus({ preventScroll: true })` in pane `giveFocus()` implementations**
   (agent `agent-model.ts`, editor, terminal) and in #3519's on-mount claim. A
   pane focusing its own input never needs to scroll ancestors. Its own
   scroller (`.agent-document`) handles its content. Defense in depth, if (1)
   is not taken.
3. **Defer the #3519 focus calls until the layout has settled**, so focus
   doesn't land on mid-transition geometry:
   - `requestNodeFocus()`: after the 0.15 s tile animation, or at least one
     `requestAnimationFrame` after commit;
   - the tab-switch `refocusNode()` (§4.5): after the reveal gate's settle
     detector reports the destination settled (`scheduleRevealLift`), not on
     a fixed two-frame delay.
4. **Belt:** a passive scroll listener on `.tile-layout` that resets
   `scrollTop`/`scrollLeft` to 0 and logs once. It turns a silent, sticky
   offset into a one-frame blip plus a log line.

## 6. Verify a fix

- Reproduce (this occurrence's shape, §4.5): a tab with 3–4 agent panes
  whose focused pane is a full-height agent pane. Leave the tab for a few
  minutes while its agents stream, then switch back to it. Repeat.
- Reproduce (insert/magnify shape): repeatedly split, insert, and
  magnify/unmagnify panes, and click between them while agents stream.
- For both, watch every tab's offset in DevTools:
  `[...document.querySelectorAll('.tile-layout')].map(t => t.scrollTop)`.
  Don't use `querySelector`: it only returns the first tab's layout (§7).
  Today a value becomes non-zero (~half the tab height). After the fix all
  must stay 0.
- Direct check: `document.querySelector('.tile-layout.animate textarea.agent-input')`
  → `.scrollIntoView()`. With `overflow: hidden` and the input out of view,
  `.tile-layout` scrolls. With `overflow: clip` it must not.
- Regression: drag and drop between panes and across tabs still works (the
  parked `placeholder-container` still has `pointer-events: none` and stays off-screen).

## 7. Correction log

- A first draft said the offset "cleared itself" after tab switches, and that a
  tab switch was the workaround. Both came from a probe that measured Tab 1's
  layout by mistake (`querySelector` returned the first `.tile-layout`). The
  user's 13:34 screenshot contradicted it, and a re-measurement of every
  `.tile-layout` confirmed Tab 2 was still at 560.8. §2 and §4.3 are corrected.
- The 10,000 px scroll range was first listed as unexplained. It's the per-pane
  `.tile-preview-container { top: 10000px }` (§3).
- §4.2 first said the composer was "at or just below" the bottom edge.
  Chromium centers only a target that is *entirely* out of view, and the
  settled composer is 3 px inside the edge, so the layout must have been
  transient at focus time. §4.5 adds the trigger for this occurrence: the
  tab-switch refocus.

## 8. Side note

The 0.57.0 host log (`agentmux-host-v0.57.0.log.2026-09-24`) reached **90 MB**
in about 8 hours, 55 MB at the previous check. Most of it is frontend noise
(`[agent-document-store] dropped chunk … reason=unknown`, `[perf] long-task`).
It's worth its own look.
