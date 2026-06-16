# Analysis: Scroll-Driven Tool-Block Collapse (replace the post-completion timer)

**Date:** 2026-06-16
**Author:** smike (agent)
**Status:** Analysis / design proposal (no code yet)
**Area:** agent-pane transcript — tool-call block expand/collapse
**Related:** `SPEC_TOOL_AUTO_EXPAND_PANEL_2026_05_16.md`, `SPEC_TOOL_HOVER_CONSOLIDATION_2026_05_28.md`, `SPEC_TOOL_BLOCK_INTERACTION_HOLD_AND_GLOB_EXPAND_2026_06_09.md`, `SPEC_AGENT_PANE_LAYOUT_REDUCER_2026_06_02.md`, `SPEC_AGENT_PANE_VIRTUALIZATION_REDESIGN.md`

---

## 1. The change requested

**Today:** a tool call renders **expanded** while running, and on completion an
auto-collapse timer fires after a few seconds, collapsing it in place — even
while it's still on screen and the user may still be reading it.

**Desired:** a tool call **stays expanded until it scrolls off the screen**, and
collapses **as it leaves the top of the viewport** (offscreen / near the top as
the transcript scrolls up). In other words: collapse should be driven by
**scroll position**, not by a wall-clock timer. While a completed tool is still
visible, it stays open; once it's scrolled past, it folds to its one-line summary
so the history stays compact.

This also aligns with a project convention seen elsewhere in the codebase: **no
grace timers** (e.g. the predictive-echo spec's "no wall-clock timer" rule). The
new behavior *removes* a timer rather than adding one.

---

## 2. How collapse works today

### 2.1 The timer is 3 s, not a minute

`frontend/app/view/agent/components/ToolBlock.tsx:82`
```ts
const POST_COMPLETION_HOLD_MS = 3000; // 1s (#988) → 5s (#1006) → 3s (2026-05-26)
```
Armed on the `active → inactive` status transition (ToolBlock.tsx:118-122):
```ts
if (isActive(prevStatus) && !isActive(s)) {
    setPostCompletionHold(true);
    const t = setTimeout(() => setPostCompletionHold(false), POST_COMPLETION_HOLD_MS);
    onCleanup(() => clearTimeout(t));
}
```
> **Note on the "after a minute" observation:** the committed code collapses after
> **3 seconds**. If the running desktop build feels like ~a minute, that build is
> off `main` (the latest Desktop portable is built from `origin/main`), or the
> perception is of a *pinned* / hovered block staying open. Either way the
> directive — *position-driven, not time-driven* — stands; this analysis targets
> the committed 3 s behavior.

### 2.2 The visual expansion decision (component-local)

ToolBlock.tsx:138-153 — the component decides what to render:
```ts
const autoExpanded = () =>
    s === "running" || s === "pending_approval" || postCompletionHold();
const expanded = () => props.pinned || autoExpanded() || userHolding();
```
- `props.pinned` — persisted (survives unmount), toggled by clicking the summary row (`onClick={props.onTogglePin}`, ToolBlock.tsx:237).
- `userHolding` — ephemeral; true while the mouse is inside an already-open block, so the timer can't collapse mid-read (ToolBlock.tsx:234-235).
- `postCompletionHold` — ephemeral; the 3 s timer. **Lost on unmount** (i.e. when the row is virtualized away).

### 2.3 The layout expansion decision (store-local) — and the divergence

The virtualization layout slice computes each row's height from a **separate**
pure mapper, `currentExpansion()`:

`frontend/app/view/agent/virtualization/expansion-source.ts:50-56`
```ts
case "tool":
    if (state.pinnedNodes.has(node.id)) return { open: true, via: "pin" };
    if (node.status === "running" || node.status === "pending_approval")
        return { open: true, via: "auto" };
    return CLOSED;   // completed tools → collapsed height
```
That file's own header (expansion-source.ts:24-31) flags the gap explicitly:
> "NOT captured here (component-local transients…): **a tool's 3 s post-completion
> hold (`ToolBlock.postCompletionHold`)** … handled by the store-layer hold timer
> / a click-time dispatch in the wiring layer, not by this pure mapper."

**So there are two deciders that already disagree during the 3 s window:** the
component paints the panel *expanded*, while the layout slice sizes the row as
*collapsed*. Measurement (`ResizeObserver` → `RowMeasured`) papers over it, but the
measured height is recorded under the *collapsed* state key, so the prefix-sum can
be briefly wrong. **This redesign should collapse the two deciders into one
source of truth** — exactly the "Phase 2" the layout-reducer spec anticipates.

### 2.4 Per-state behavior today

| Status | Expanded while…? | On completion |
|---|---|---|
| `running` / `pending_approval` | always | n/a |
| `success` / `failed` | — | 3 s hold, then collapse (✗ + red border flags failures on the collapsed row) |
| `denied` / `canceled` / `awaiting_answer` | — | collapse immediately |
| pinned (any) | always (pin overrides) | stays expanded |

Collapse state ownership: `pinnedNodes` / `collapsedNodes` are `Set<string>` in
`documentState` (`frontend/app/view/agent/types.ts`) — **persisted, survive
virtualization unmount**. `postCompletionHold` / `userHolding` are component
signals — **ephemeral, reset on remount**.

---

## 3. The scroll/virtualization infrastructure to hook into

The transcript is **hybrid-virtualized** (`SPEC_AGENT_PANE_VIRTUALIZATION_REDESIGN.md`):
a virtualized head (absolute-positioned, windowed) + an unvirtualized streaming
buffer (last ~50 nodes in normal flow). Key facts for this work:

- **Every row has a known position.** `computeLayoutView` builds a prefix-sum of
  `{ start, height }` for **all** ordered rows; the visible window is a binary-search
  slice (`reducer.ts:windowRangeOf`). So "is row R above the viewport top?" is
  computable as `row.start + row.height < scrollTop + threshold` for *any* row,
  rendered or not — no DOM/IntersectionObserver needed for the virtualized head.
- **Heights are state-keyed.** `RowHeight { collapsed?, expanded? }` — both the
  collapsed and expanded heights are measured/estimated and stored
  (`reducer.ts:layoutHeightFor`). Collapsing a row to a *known* height is a
  deterministic prefix-sum change — no remeasure required.
- **Offscreen rows unmount** (`AgentDocumentVirtualList.tsx:587-590` slices to the
  window). This is why a scroll-off latch **cannot** live in the component — the
  component is gone exactly when we need to record "this scrolled off."
- **Scroll/anchor machinery already exists:** `handleScroll` toggles
  stick-to-bottom by proximity (`anchor.ts:isNearBottom`, threshold 200 px);
  near-top triggers paginate-older with `captureTopmostAnchor` /
  `restoreScrollFromAnchor` to **prevent scroll jumps on prepend**
  (`AgentDocumentVirtualList.tsx:491-575`). The same anchor primitive is what we
  need to keep the viewport stable when a row above the fold collapses.
- **Expansion already flows into layout via commands:** `ExpansionResolved`
  dispatches on expansion change and triggers a relayout
  (`AgentDocumentVirtualList.tsx:239-248`; `agent-pane-layout-store.ts:81-95`).

---

## 4. Proposed model: position-driven, **latched** collapse

### 4.1 The rule

A completed, unpinned tool is **expanded by default** and collapses **once, when
its bottom edge scrolls above the viewport top**, then **stays collapsed (latched)**.

```
expanded(tool) =
      tool is running | pending_approval        // live → open (unchanged)
   || pinned(tool)                               // explicit override (unchanged)
   || userHolding(tool)                          // mouse-inside hold (unchanged)
   || ( completed && NOT scrolledOff(tool) )     // NEW: open until it leaves the top
```

- **Completed & still on/below the fold →** open. (Removes the 3 s timer's early
  collapse — the user keeps reading it.)
- **Completed & scrolled above the top →** add `tool.id` to a `scrolledOff` set →
  collapsed, and it **stays** collapsed on scroll-back (latched).
- **Completed off-screen above when it finished** (user had scrolled down past it
  while it ran) → it's already above the top → collapsed immediately. Correct.
- **Completed off-screen below** (user scrolled up; new tool finished below the
  fold) → not above the top → stays expanded; visible & open when the user
  returns to the bottom. Correct.

### 4.2 Why **latched** (one-way), not "expanded iff currently in view"

A continuously-recomputed "expanded iff visible" rule creates a **feedback loop**:
expansion changes height → height changes the prefix-sum → that changes which rows
are above the top → which changes expansion… potentially oscillating, and
re-expanding on scroll-back would thrash row heights right under the user's cursor.

Latching (`expanded → collapsed`, once, per node id) makes the decision
**monotonic**: a row leaves the top at most once, collapses once, and is done.
This kills the loop and matches the user's words ("stay expanded **until** off
screen, **then** collapse"). Re-expanding is then an explicit click (pin), which
is the existing affordance.

### 4.3 Where the latch lives

`scrolledOff` must be **pane-level persisted state**, not a component signal —
because the row unmounts the instant it scrolls off. Natural home: a new
`Set<string>` in `documentState` alongside `collapsedNodes` / `pinnedNodes`, fed
into `currentExpansion()` via the existing `ExpansionInputs`. Then **one** mapper
decides expansion for both the layout heights *and* the visual render — closing
the §2.3 divergence.

### 4.4 Single source of truth

Refactor so `ToolBlock` no longer owns the expand decision:
- Delete `POST_COMPLETION_HOLD_MS`, `postCompletionHold`, and its `createEffect`
  (ToolBlock.tsx:82-83, 109-124).
- Extend `currentExpansion()` (expansion-source.ts) for tools:
  ```ts
  case "tool":
      if (state.pinnedNodes.has(node.id)) return { open: true, via: "pin" };
      if (running || pending_approval)    return { open: true, via: "auto" };
      return state.scrolledOff.has(node.id)
          ? CLOSED
          : { open: true, via: "auto" };   // completed-but-not-yet-scrolled-off → open
  ```
- `ToolBlock.expanded()` reads the resolved expansion (passed as a prop / read
  from the same store) `|| userHolding()`. `userHolding` stays component-local
  (it's a true transient and only matters while mounted/visible).

---

## 5. The hard part: scroll-height stability

Collapsing a row **above** the viewport shrinks total height above `scrollTop`. If
uncompensated, in-viewport content **jumps upward** by the height delta. Two
regimes:

1. **Stick-to-bottom engaged (active streaming, the common case).** `scrollTop`
   is pinned to max; rows collapsing above the fold just reduce `scrollHeight`
   and we stay pinned. **No visible jump** — this case is naturally stable.
2. **User scrolled up reading history.** Collapsing above-fold rows *must*
   decrement `scrollTop` by the summed collapsed-vs-expanded height delta to keep
   the viewport visually stationary. Reuse the existing anchor primitive
   (`captureTopmostAnchor` / `restoreScrollFromAnchor`,
   `AgentDocumentVirtualList.tsx:491-575`): capture an anchor on a row that stays
   in view, dispatch the collapse(s), restore scroll from the anchor's new
   prefix-sum start. The machinery is already there for paginate-older; this is
   the same shape.

**Trigger point.** Hook the latch into the existing `handleScroll`
(`AgentDocumentVirtualList.tsx:455`), which already runs on every scroll and has
`scrollTop` + the layout view. On each scroll, scan rows whose
`start + height(expanded) < scrollTop + COLLAPSE_MARGIN_PX` that are tool nodes,
completed, unpinned, not already in `scrolledOff`; batch-add them to `scrolledOff`
in one dispatch (one relayout), with anchor compensation per regime above.
- Use the **expanded** height in the threshold so the decision is computed against
  the row's pre-collapse extent (monotonic; avoids a row oscillating around the
  boundary).
- `COLLAPSE_MARGIN_PX`: 0 = collapse exactly when fully above the top; a small
  positive value = "near the top" (collapse just before fully gone, a gentler
  feel, matching "offscreen **or near the top**"). Tunable; start at 0–24 px.

**Tall tools** (taller than the viewport): keying on the **bottom** edge
(`start + height`) means a tool whose top has scrolled off but whose bottom is
still visible stays expanded until it's *entirely* above the fold — so the user is
never yanked mid-read. Correct by construction.

---

## 6. Edge cases / decisions

| Case | Behavior |
|---|---|
| Completed tool still fully/partly visible | **Expanded** (new). The headline win. |
| Tool scrolls entirely above the top | Collapse once, **latched**. |
| Scroll back up to a latched tool | Stays collapsed; **click to expand** (pins). No auto-thrash. |
| Tool finishes while already above the top | Collapsed immediately (already in `scrolledOff` criteria on next scroll/relayout). |
| Tool finishes below the fold (user scrolled up) | Expanded; visible when user returns to bottom. |
| Pinned tool | Always expanded; **never** auto-latched. Pin still wins. |
| `denied` / `canceled` / `awaiting_answer` | Keep today's immediate-collapse (don't hold these open on screen). |
| `failed` | Same as `success`: stays open while visible, collapses on scroll-off (✗ + red border on the collapsed row, unchanged). Confirm this is wanted vs "errors stay expanded." |
| Streaming buffer rows (last ~50, in-flow near bottom) | These are at/near the bottom = visible = expanded. They only latch once they've migrated up into the prefix-sum head and crossed the top — handled uniformly by position. |
| `userHolding` (mouse inside) when it reaches the top | Hold wins while the mouse is in; latch on the scroll after `mouseleave`. (Or: don't latch a row the pointer is inside — minor.) |

**Persistence question:** should `scrolledOff` persist across reload/restore (like
the snapshot in `SPEC_AGENT_PANE_STATE_PERSISTENCE_2026_05_15.md`)? Recommendation:
**no** — on a fresh load, completed tools above the restored scroll position can be
seeded as collapsed directly from their position, and anything in view renders
expanded. Keeping `scrolledOff` session-ephemeral avoids growing a persisted set
unboundedly and matches "collapse is a scroll artifact, not a saved preference."

---

## 7. Implementation sketch (files)

1. **`frontend/app/view/agent/types.ts`** — add `scrolledOff: Set<string>` to
   `DocumentState` (+ a `markScrolledOff(ids)` mutator). Widen
   `ExpansionInputs` (expansion-source.ts:40) to include it.
2. **`expansion-source.ts:50-56`** — completed unpinned tool: open unless in
   `scrolledOff`. (Single source of truth; fixes the §2.3 divergence.)
3. **`AgentDocumentVirtualList.tsx`** — in `handleScroll` (and once after relayout/
   resize), compute newly-off-top tool rows from the layout view's prefix-sum and
   batch-dispatch them into `scrolledOff`; wrap the dispatch in anchor
   capture/restore when **not** stuck to bottom (reuse §5 primitives). Add
   `COLLAPSE_MARGIN_PX`.
4. **`ToolBlock.tsx`** — delete the timer (`82-83`, `109-124`); `expanded()` reads
   the resolved expansion (prop/store) `|| userHolding()`. Keep pin + hover-hold.
5. **Specs** — supersede the "post-completion hold" sections of
   `SPEC_TOOL_AUTO_EXPAND_PANEL_2026_05_16.md` §4.2 and note the unification as the
   `currentExpansion` "Phase 2" the layout-reducer spec anticipated.

Roughly: one new `documentState` set + mutator, a ~15-line scroll-scan with anchor
compensation, a 3-line change in the pure mapper, and a deletion in ToolBlock.

---

## 8. Risks / open questions

- **Scroll-jump on history scroll-up (§5 case 2)** is the main risk. The anchor
  primitive exists but must be wired correctly; needs a running-app check (the
  desktop build can't verify a headless diff). Easy to get subtly wrong → visible
  jump when many rows collapse at once.
- **Batching:** collapse all newly-off-top rows in **one** dispatch per scroll
  frame, not one per row, to avoid N relayouts and N anchor restores.
- **`failed` policy:** confirm errors should fold on scroll-off like successes
  (current behavior) vs. staying expanded as a louder signal. (My default:
  fold-on-scroll-off; the ✗/red row already flags it. Flagging for your call.)
- **Margin feel:** `COLLAPSE_MARGIN_PX` (collapse exactly at the top vs. slightly
  before) is a taste tune — set after seeing it live.
- **Re-expand affordance:** latched-collapsed relies on click-to-pin to re-open. If
  you want "scroll back up → re-expand automatically," that reintroduces the
  feedback loop (§4.2) and is not recommended.

---

## 9. Recommendation

Adopt the **position-driven, latched** model with a **single expansion source**
(`currentExpansion`) and **anchor-compensated** collapse. It removes a timer
(aligns with the no-grace-timers convention), fixes the existing dual-decider
divergence the layout-reducer spec already wants closed, and uses infrastructure
that already exists (prefix-sum positions, state-keyed heights, scroll anchors).

Smallest correct first cut: ship the **stick-to-bottom case first** (no anchor
math needed — the dominant case during live streaming), behind the unified
mapper; then add the **scrolled-up anchor compensation** as the second increment.
That sequences the only real risk (scroll-jump) into its own verifiable step. If
you want, I can turn this into a spec + start the Phase-1 cut.
