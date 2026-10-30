# Agent pane: the latest message stays tight against the bottom edge, always

**Status:** active — approved by the operator 2026-10-29 ("approve all recommended"); Phase 1 (the spacer removed, the resize snap, the recorder's tight class and cause log) in PR #PENDING; Phase 2 (docked rows ease their height) and Phase 3 (the kill-switch path) are open.
**Date:** 2026-10-29.
**Requested by:** the operator, 2026-10-29: "we need some refining to the scroll on agent panes: I am also noticing overshoot in the agent pane, where an agent will be outputting tool calls and messages, and appears ok, but then spontaneously, a large blank space appears, that the agent only slowly fills up with new messages. Another thing, we also have cases where if we resize the agent pane, a gap can grow at the bottom if the pane is made shorter than taller again. so we want to keep the latest message tight against the edge no matter what, robust thinking".
**Operator's answer on the shrink, 2026-10-29:** "ah, for moving down, yes, on pane resize, thats likely the only way. but during normal agent converstaion, we'd rather simply not overshoot during normal converstions".
**Author:** Loap-3@narko (Kanban #11293).
**Amends:** `SPEC_AGENT_PANE_ONE_WAY_FLOW_2026_10_07.md` (R2's "visible rows never move down", R3 the bottom spacer, §7 decision 1 "accept blank space at the bottom"). Everything else in that spec stands.
**Related:** `SPEC_CONTENT_RESIZE_CONTRACT_2026_08_31.md` §2 (the shrink hold), `SPEC_AGENT_PANE_SCROLL_FOLLOW_STATE_MACHINE_2026_09_24.md`, Camper-2's `SPEC_ZOOM_ANCHOR_AND_AUDIT_2026_10_29.md` (#11019).

Line references are against `origin/main` at `768aad579`. "VL" is `frontend/app/view/agent/virtualization/AgentDocumentVirtualList.tsx`, "OWF" is `frontend/app/view/agent/scroll/one-way-flow.ts`.

## 1. Summary

Both symptoms are one mechanism working as designed: the bottom spacer (OWF `spacerFor`, R3). Whenever the content below the reader gets shorter, or the scroller gets taller, the browser clamps `scrollTop` and the rows slide down. R2 then scrolls forward to put them back where they were, and the room that needs is made by a blank spacer at the bottom. The spacer is only used up by new content, and an agent's next row takes 2 to 4 s (one-way flow spec §2.2), so the reader sees a blank the size of whatever shrank. The spec chose that on 10-07 ("Blank space at the bottom until new content fills it ... Recommended: accept") so that text would never move down. The operator now prefers the opposite trade.

**The fix is two layers.** (1) The backstop: delete the spacer while following, so whatever still shrinks on screen lands tight (instant) and is never paid for with blank room. The invariant T: while the pane follows and the user is not touching it, after every observation the spacer is 0 and the newest content is at the bottom edge or on its way there; there is never room below it. (2) The operator's preference (§4): in a normal conversation nothing should shrink on screen in the first place, so the sources are removed or smoothed one by one, and the backstop only catches the rest. A pane resize (and the other user-initiated changes) is allowed to move text, as the operator said.

AgentY's first read is right on the spacer and wrong on the shrink hold: on the default path the hold does nothing (`holdBufferHeight` is only called from the content observer, which returns early when `oneWay`, VL:1092). It only matters under the kill switch `agent:onewayflow=false`.

## 2. Root causes

### 2.1 The large blank that appears mid-turn

Anything that makes the content below the reader shorter, or the scroller taller, while the pane follows. The one-way spec's example is a finished tool's preview collapsing (81% of the shrinks recorded on 09-03, before I traced the held-open rule: see §4, source 1), and its other example is a docked panel leaving. Which of these produced the operator's large blank is not established from code alone (§4 says what I found and what the build must measure); the mechanism below is the same for all of them. In one `onResize` (OWF:286):

1. The browser has already clamped `scrollTop`; every row above moves down by the shrink `d`.
2. `largestDownwardMove` (OWF:50) finds `d`; `wanted = live + d` (OWF:345).
3. `spacerFor` (OWF:77) returns `ceil(wanted + clientHeight − scrollHeight)` = `d`; `setSpacer(d)` makes `d` px of empty element after the streaming buffer (`_document.scss:354`).
4. The write at OWF:356 moves `scrollTop` forward by `d`. The reader sees the old picture plus `d` blank pixels.

It looks "spontaneous" because it arrives when something the agent did lands (a tool result, a panel ending), not when the user does anything, and it fills "slowly" because the formula uses new content up first and the next row is seconds away. Its size is the size of whatever shrank, up to a tool preview's cap (`previewCapPx`) or a docked panel's height.

Reproduced on the geometry model the existing tests use (`one-way-flow.test.ts` `Model`, run on `origin/main`): a 240 px preview collapsing at the bottom leaves 240 px blank (`spacerPx() == 240`). The existing tests pin this on purpose: "a visible row shrinking at the bottom keeps everything above it in place and leaves room" (line 282) and "a taller viewport (the working row leaving) keeps the content in place" (line 325).

### 2.2 The gap after resizing shorter, then taller

The pane height is not treated as a user-initiated change; only width/zoom is (OWF:310 `cw !== this.lastWidth` resets the baseline and the spacer). The spec's own §1 lists "a pane resize" as exempt, so this is a code gap, not a decision.

- Shorter: the viewport shrinks, `wanted = live`, the formula returns less spacer (or 0), the follower scrolls forward to the new bottom. Tight.
- Taller: the browser clamps `scrollTop` down by `Δ`, the rows move down by `Δ`, R2 compensates, R3 makes a `Δ` px spacer. A `Δ` gap at the bottom that stays until the agent writes `Δ` px of new content.

On the model: a pane made 200 px shorter and 200 px taller leaves a 200 px spacer (199 px blank at fractional rounding); a 150 px growth from a tight bottom leaves 150. So the gap is not specific to "shorter then taller": any growth of the scroller while following does it. Shorter-then-taller is just when it is noticed, because the first step shrinks the visible blank away and the second brings it back.

### 2.3 What is not the cause

- The shrink hold (`min-height`): off on this path (see §1).
- Height estimates (`estimateNode`, `previewCapPx`): rows in the live tail are measured, not estimated, and a head row measured shorter than its estimate shrinks content wholly above the viewport; the browser's clamp then moves nothing visible (R2 table, row 5). I found no path where an estimate leaves blank at the bottom of a following pane. If the live check (§6) shows one, it is a separate fix.
- The follower: it only moves forward toward real content; with τ = 60 ms it reaches a new bottom in about 100 ms, which is the pane catching up, not blank.

## 3. The invariant, in one place

**T (tight):** while FOLLOWING and the user is not touching the pane, at the end of every `OneWayFlow.onResize`: `spacerPx == 0`, and `scrollTop` is the follower's position toward `scrollHeight − clientHeight`, which only differs from it by content that arrived below the viewport and is being scrolled to.

It is enforced in `OneWayFlow.onResize`'s active branch, nowhere else:

- `wanted` (the R2 compensation for growth, which always has room because growth made it) is clamped to the true maximum: `min(live + d, sh − spacerPx − ch)`.
- `setSpacer(0)` replaces the `spacerFor` call. The settle branch (OWF:321) already does exactly this for 600 ms after a send; the change is that it becomes the only behaviour.
- The detached branch keeps nothing: with no spacer made while following there is none to keep (the code at OWF:371-377 and `userScrolled` OWF:267 go).

What is removed: `spacerFor`, `SpacerInput`, `spacerPx`/`setSpacer`/`spacerHeight`, `bottomSpacerRef`, the `.agent-document-bottom-spacer` element and its CSS, `userScrolled`'s spacer release (VL:1260). `commit()` and `settleOnShow()` stay for what they do besides the spacer: snap to the bottom instead of easing.

**V1 becomes V1':** no visible row moves down while following, except when the content below it got shorter or the scroller got taller, where it moves down by exactly that amount, once, and the bottom stays tight. Growth above or inside the view is still compensated (R2), because growth makes its own room. The recorder (`one-way-recorder.ts`) must classify those two causes as expected "tight shifts" and keep counting everything else as a violation; its V2 ("blank shown then taken back") becomes "any blank below the last row while following" which is now a hard failure.

### 3.1 Every event, one rule

| Event | Today | With T |
|---|---|---|
| A row shrinks at the bottom | spacer = shrink | rows slide down by the shrink; bottom tight (§4 removes or smooths the agent-initiated sources first) |
| Working row, dock or panel leaves (scroller taller) | spacer = growth | the same |
| Pane made taller (drag, window, layout) | spacer = growth | the same |
| Pane made shorter | follower eases to the bottom | snap to the new bottom in the same callback (new: `lastHeight` mirrors `lastWidth`; a latest message cut off by the resize must not wait 100 ms for the ease) |
| Zoom | baseline reset, ease | baseline reset, snap |
| Hold timer (350 ms) | not used on this path | deleted with the kill-switch path (§7) |
| Hide / show | `settleOnShow` clears the spacer, lands at the bottom | unchanged |
| Reload / reopen | first observation | first observation is tight; the scroll memo restore of a following pane must land at the bottom (`useScrollMemo`) |
| Composer grows/shrinks, queued panel | commit window | no window needed; same rule |
| New content below | follower eases | unchanged |
| User scrolls up | detach; browser anchoring keeps the reader's place | unchanged |
| Re-engaging follow (scroll to bottom, jump button) | spacer released | nothing to release |

## 4. Normal conversation: nothing shrinks on screen (the operator's answer)

The operator, 2026-10-29: moving text down is fine on a pane resize; "during normal agent conversation, we'd rather simply not overshoot". Read as AgentY relayed it: in a conversation, while following, no blank space and no visible jump down; remove the cause, and let whatever still shrinks on screen fall back to instant-and-tight (never blank).

**Which changes are the user's, and so may move text** (instant, tight, no ease): a pane resize, zoom, sending a message, answering or denying a question or decision, the composer growing or shrinking, the user collapsing, expanding or pinning a tool or toggling "Taller/Shorter preview", `/clear`, switching to the pane's tab. Everything else is agent-initiated and falls under the rule below.

**What I could and could not establish.** I read every path that can make the visible bottom area shorter or the scroller taller without the user doing anything. I could not measure live (no dev build in quota mode), and from code alone I found no large, common, agent-initiated shrink on screen: the candidate the one-way spec and AgentY name (a finished tool's preview collapsing) is already prevented by an existing rule. So the table below is a code reading, each row marked with how sure it is, and the build's first job is to measure what is left (§6).

| # | Source (agent-initiated) | What shrinks | Removed by | What is left over |
|---|---|---|---|---|
| 1 | A finished tool's preview collapses to its result | content, in view | **Already removed, which is AgentY's option (a).** A tool that finishes live on screen is held open (`heldOpenNodes`, `disclosure.ts:73`, set by `ToolBlock`'s active to inactive transition) until its bottom is above the viewport's top (`held-release.ts:123`), where the collapse is wholly above the viewport and moves nothing visible (R2 table, row 5). The 09-03 attribution (81% tool rows) predates my reading of this; whether it predates the rule itself I did not check. | A tool that finished while the pane was hidden is not on screen then and is held on show only if it is (`AwayArrivals`); a denied or canceled tool closes at once, but that is the user's own deny or stop. Sure from code. |
| 2 | The open tool box changes height: the live log swaps for the result, an async renderer finishes about 150 ms later, `ToolOverlayLog`'s own branch swap | content, in view, bounded by the preview cap | Not removable without slack: holding the box at its tallest height leaves an empty strip inside the last row, which looks like the bug in a smaller box. `resize-contract.ts` already eases these height changes (`beginHeightContinuity`, 160 ms, up to 2000 px), so the shrink is a smooth slide whose every frame is tight. | A slide of at most the cap, 160 ms, no blank, no jump. Under reduced motion it is instant. Not measured: how often and how big. |
| 3 | Markdown re-renders from plain text to highlighted (about 90 ms after the last token) | the streaming message | Nothing to do: 0 of 351 attributed shrinks were markdown (09-03, `SPEC_CONTENT_RESIZE_CONTRACT_2026_08_31.md` §5). | Fractions of a line. |
| 4 | The working row leaves | the scroller grows | **Already removed, which is AgentY's option (b).** The row is a normal-flow sibling below the scroll region and it does not leave when the turn ends: `workingRowVisible` stays true once `sessionStats` exists, and the line becomes "Worked · Ns" and "stays visible as a turn delimiter until the user sends the next message" (`AgentFooter.tsx:98`, `useWorkingIndicator.ts`). It leaves at the next send, which is the user's. | It appears at the first turn of a session (the scroller shrinks, content moves up, allowed). The "Working" and "Worked" lines must be the same height: a test in the build pins it. |
| 5 | A docked row ends by itself: an `ActivityDock` background task finishes, `ApiRetryRow` (retry over), `AgentPreparingRow` (the container start is over), a recovery banner or session notice clears | the scroller grows by the row's height | These show state, so they cannot be removed. Give each an exit transition that eases its own height to 0 (the grid-rows 1fr to 0fr technique, or the same `resize-contract` module), so the scroller's height eases over 160 ms and the tail stays tight every frame. Not a shared wrapper: `AgentBottomPanels` renders a fragment on purpose, each panel a direct flex child of `.agent-view-zoomed` (its own header comment, §3.3). | A slide of one panel's height (a dock with several tasks is the biggest), 160 ms, no blank, no jump. This is the source I would expect to be largest for a user running background tasks. |
| 6 | A head row measured shorter than its estimate; a row migrating tail to head | content wholly above the viewport | Nothing: the clamp moves no visible row. | None. |
| 7 | A held tool released after scrolling off the top | content wholly above the viewport | Nothing: same. `OneWayFlow` already runs the release first (`prepare`) so any move is compensated in the same frame. | None. |

Fallback for anything else: it falls under T and lands instantly and tight, and the live attribution (below) names it so it can be added to this table.

**Cost and risk.** Rows 1, 3, 4, 6, 7 need no change; row 2 none beyond what exists; row 5 is the only new motion work, and it is small and local to the panels. The cheap part is also the part that makes the claim checkable: the shrink sources are named in the recorder (§6), so "nothing shrinks on screen in a normal conversation" becomes a number from the soak and from dogfooding, not an argument.

## 5. One place owns the scroll write

While FOLLOWING, `OneWayFlow` is the only writer of `scrollTop` for the transcript (old spec R4, unchanged): the follower's frames, `jumpToBottom`, `settleOnShow`, and now the height-change snap. The other writers (`scrollToNode`/`scrollToTrueBottom` for the non-one-way path, `keep-on-screen.restoreOnScreen`, the older-history restore at VL:1485, `scroll-memo`) act on a detached reader or at mount.

Camper-2's zoom anchor (#11019) writes `scrollTop` through `scrollToNode`, and by their spec "skips when the pane is following or hidden". That is compatible: the anchor owns the write for a detached reader, `OneWayFlow` for a following one, and a zoom of a following pane keeps the latest message tight (§3.1). Proposed handshake, to confirm with Camper-2: the zoom hook asks `follow.following()` once at capture time and, if true, does nothing and leaves the snap to `OneWayFlow`; a pane that stops following during the settle loop abandons the restore. I will message them once.

## 6. Verification

**Unit (geometry model, `one-way-flow.test.ts`).** Flip the three tests that pin the blank and add the rest:

1. a 240 px collapse at the bottom (the backstop, for a shrink nothing prevented): `spacerPx() == 0`, `scrollTop == max`, the last row's bottom is the viewport bottom, rows above moved down by exactly 240.
2. the next content after a collapse: nothing moves, the content arrives at the bottom.
3. a taller scroller (60 px): no spacer, tight.
4. shorter then taller (200 px each): no spacer, tight after each step; and a repeated cycle never accumulates.
5. a shorter scroller snaps in the same callback (the write count, no ease).
6. growth inside the view is still compensated (the existing V1' tests stay).
7. user touching: nothing moves (unchanged); re-engage: tight.
8. a collapse while detached: nothing written, the browser's anchoring keeps the reader's place (unchanged).

**Component (`AgentDocumentVirtualList.resize.test.tsx` style, fake ResizeObserver).** The same two symptoms through the real list: stream rows, collapse a tool, resize the scroller shorter then taller. Plus the prevention: a tool finishing on screen stays open (row 1); "Working" and "Worked" rows are the same height (row 4); a docked row ending eases its height and never blanks (row 5).

**Recorder / soak.** `one-way-recorder.ts` learns the "tight shift" class; `one-way-soak.mjs --strict` gets a bottom-blank check each frame (blank below the last row while following > 0.5 px is a violation) and a scripted collapse and resize. Acceptance: zero blank frames in a 30-minute 3-pane soak.

**Attribution (new, permanent).** Every tight shift (the two classes V1' allows, plus any on-screen shrink) is logged with its cause: the node id and type that got shorter (the existing per-node snapshot diff in `shrink-trace.ts`, which today only runs on the old pin path), or the docked panel whose height changed, and its size. `muxlog fe grep` then gives the real distribution from dogfooding. This is what decides row 2 and row 5 above and any source I missed. The soak gets a per-cause histogram.

**Live check (in the build, in its own `task dev`, never the operator's instance).** Two recordings, before and after: (1) a tool preview collapsing mid-turn; (2) the pane dragged shorter, then taller. The model reproduces both symptoms with the exact sizes above, but I have **not** recorded the live clips for this spec (a dev build is heavy on narko in quota mode); recording the "before" clips is the first step of the build, and they double as the baseline for the soak.

## 7. Plan

| Phase | What | Notes |
|---|---|---|
| 1 (the build, #11294) | §3: delete the spacer while following, the height-change snap, recorder classes and the cause attribution (§6), tests, the two recordings | one PR; small: OWF, VL (spacer ref and release), `_document.scss`, recorder, shrink-trace wiring, tests |
| 2 (same PR if small, else the next) | §4 row 5: docked rows ease their height when they end by themselves; row 4's same-height test | after the attribution shows which panels matter |
| 2b (only if the attribution asks) | §4 row 2: a policy for tool-box height swaps beyond the existing 160 ms ease | measured first |
| 3 (separate, AgentX's call) | Delete the kill-switch path (`agent:onewayflow=false`): the glide, the shrink hold, `pin:*` paths | the old spec's Phase 3; today the hold still makes blank under the switch |

## 8. Decisions

Decided by the operator (2026-10-29): moving text down on a pane resize is fine; in a normal conversation prefer not shrinking at all over repairing after.

Left for the operator:

1. **The treatment in §4 as a whole:** rows 1, 3, 4, 6, 7 already hold or need nothing; row 2 stays a 160 ms tight slide (not an empty strip inside the box); row 5 gets exit easing for docked rows. If the operator would rather have no motion at all for row 5, the alternative is instant and tight (still no blank).
2. **Kill switch path:** AgentX's call, as its own PR after phase 1 has run clean for a week (delete `agent:onewayflow=false`, the glide, the shrink hold).

Agent: Loap-3@narko
