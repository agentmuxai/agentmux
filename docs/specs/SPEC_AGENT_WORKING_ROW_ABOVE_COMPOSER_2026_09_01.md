# Working row: stand down on promotion, and sit above the composer

**Date:** 2026-09-01
**Status:** Implemented
**Supersedes (in part):** `SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY_2026_07_24.md` §3.2,
`SPEC_AGENT_WORKING_ROW_SCROLLBAR_GAP_2026_08_06.md` (entirely — see §4)

Two changes to `AgentWorkingRow`, both about the same thing: making the agent
pane's bottom stack say each fact exactly once, in the place the eye already
is.

---

## 1. The working row stands down when the dock takes over

### Problem

Long-running shells are auto-promoted into the ActivityDock after
`TOOL_PROMOTION_MS` (or immediately, for a declared `sleep` — the case that
prompted this). Promotion is the moment the work stops being *"the pane is
blocked on this"* and becomes *"this is running in the background, tracked
over there, with its own live countdown."*

The `Working…` row kept rendering straight through that transition. So the
pane showed two rows for one fact, and the louder of the two asserted the
opposite of what had just happened: `Working…` says the pane is busy waiting,
when backgrounding the task is precisely what made it not busy.

`toolPromoted` was already plumbed into `AgentWorkingRow`, but it only
suppressed the *label* (`AgentFooter.tsx` — `props.currentTool &&
!props.toolPromoted`), never the row. The spinner and elapsed timer stayed.

### Fix

`workingRowSupersededByDock` (`activity/working-row-supersession.ts`) gates
`workingRowLoading`. With no other reason to be visible — `sessionStats` is
cleared at turn submit, so a stale "✓ Worked · Ns" can't stand in — the row
hides outright.

**Deliberately narrow.** It suppresses only the plain "a promoted tool is
running and nothing else is happening" case. Everything the dock has no
vocabulary for keeps the row:

| State | Why the dock can't express it |
|---|---|
| launch activity | the pane isn't up yet; the dock says nothing about launch |
| `Interrupting` | "Stopping…" is about the turn, not the tool |
| `waitingReason` (rate limited / retrying) | a real condition with no dock representation — and the one the user most needs to see |
| compacting | not a tool |
| reconnecting | not a tool |

The predicate is a pure function with its own tests rather than a closure
inside `agent-view.tsx`, because the interesting part *is* that exception
list: the failure mode of this feature is silently swallowing one of those
five states, and that is exactly what a test can pin.

---

## 2. The row moves to sit directly above the composer

### Problem

Reading bottom-up, the pane's stack put this turn's own live status
*furthest* from the composer, above the dock's background tasks — the
opposite of how attention narrows.

### Fix

DOM order is now:

```
ActivityDock          ← long-running background tasks
AgentWorkingRow       ← what this turn is doing right now
AgentComposerStrip    ← where you type
```

Bottom-up, scope narrows: what's running in the background → what's happening
this instant → where you act.

---

## 3. What this deleted

The row previously floated over `.agent-document-scroll-region`'s bottom edge
as an absolutely-positioned overlay (07-24 §3.2), so the message list's
scrollbar could run that box's full height. That one decision required a
surprising amount of scaffolding to hold up — all of which the move removes,
because **a row that is not inside the scroll region cannot overlap its
scrollbar or its content in the first place.**

| Removed | What it was compensating for |
|---|---|
| `workingRowHeight` signal + ref-callback `ResizeObserver` (`agent-view.tsx`) | measuring the overlay so the document could reserve space under it |
| `--agent-working-row-height` custom property | shipping that measurement across two component boundaries via the CSS cascade |
| `.agent-document`'s `padding-bottom: calc(… + var(…))` | keeping the last message from hiding beneath the overlay at true bottom |
| `workingRowHeight` prop through `AgentDocumentView` → `AgentDocumentVirtualList` | — |
| that prop as a third stick-to-bottom dependency | overlay growth changing effective content height with no node/layout change (reagent P1 on #2292) |
| `.agent-working-row-backdrop` (element, both style variants, and its `<Show>`) | coloring the scrollbar gutter the overlay had to stay inset from (08-06, entirely) |
| the anchor's `right: var(--agent-document-scrollbar-width)` inset | stopping the row's opaque background from painting over the native scrollbar |
| the anchor's `z-index: 2` + `pointer-events: none` / row-level `auto` | paint order against `.agent-document`, and stopping the anchor's empty margin from swallowing clicks meant for the message list |
| `.agent-view--working-row-visible` class + its margin rule | closing the gap between the row's color and the panel directly below it |

Net: **−32 lines** across the two scroll/virtualization files alone, plus two
CSS blocks, with no behavior traded away.

### Scroll-follow is still correct

The one real question this raises. As a normal-flow sibling, the row changes
the scroll region's **`clientHeight`** when it appears or disappears — not its
content height.

`AgentDocumentVirtualList`'s `clientHeight` `ResizeObserver` already re-pins on
exactly that, and was written for exactly this family of siblings: its own
comment names "the retry bar, AgentDecisionPanel, AgentQuestionPanel, or
PendingMessagesPanel appearing/growing mid-turn — all normal-flow,
flex-shrink:0 rows." The working row simply joins that list. It needs no
tracked height signal of its own, which is why removing the dependency above
is safe rather than a regression waiting to happen — the observer was
introduced specifically to stop the per-panel-signal whack-a-mole that the
overlay's own tracked dependency was an instance of.

---

## 4. Note on the superseded specs

`SPEC_AGENT_WORKING_ROW_SCROLLBAR_GAP_2026_08_06.md` is superseded in full:
every element it introduced existed to serve the overlay geometry, and the
bug it fixed (the scrollbar gutter showing plain pane background instead of
the row's color) cannot occur for a row outside the scroll region.

`SPEC_AGENT_PANE_SCROLL_FOLLOW_AND_STATUS_OVERLAY_2026_07_24.md` is superseded
only at **§3.2** (the overlay itself). Its §2 scroll-follow analysis and the
`clientHeight`/content-resize observers remain load-bearing and unchanged —
this change leans on §3.3's normal-flow-sibling handling rather than
contradicting it.

---

## 5. Verification

- `workingRowSupersession.test.ts` — 9 tests: the promotion case, the
  no-promotion case, all five keep-the-row exceptions, and two guards on
  reading the optional `waitingReason` off a non-`Streaming` variant.
- `npx tsc --noEmit` clean.
- Full `frontend/app/view/agent` suite green.
