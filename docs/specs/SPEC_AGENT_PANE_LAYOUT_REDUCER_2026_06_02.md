# Agent-Pane Layout State Machine — unify zoom + virtualization + tool-expansion into one reducer

**Status:** Design resolved (§11); **Phase 0 implemented** — pure slice core + store + tests (no render-path wiring yet)
**Date:** 2026-06-02
**Author:** AgentA
**Tracking:** issue #1235 (the drift this eliminates); builds on PRs #1231/#1233/#1234
**Pattern:** follows `docs/specs/frontend-reducer-conventions-2026-05-03.md` (Slice #11)

---

## 1. Why this spec exists

Three separate bugs this cycle — overlap-under-zoom (#1231), stuck-estimate rows
(#1233), and the expand/collapse crash + measurement drift (#1234, #1235) — are
all **the same disease wearing different clothes**: the agent-pane has **three
uncoordinated sources of truth** for the only two questions that matter to layout:

> *How tall is row N (in its current state)? Where does it sit?*

| Truth source | Owns | Where | Problem |
|---|---|---|---|
| **TanStack `itemSizeCache` / `measurementsCache`** | measured heights + computed `start/size/end` | `@tanstack/virtual-core` (imperative, ResizeObserver-driven) | Incremental, mutated mid-reflow; caches transient garbage (`0`, `784`) that doesn't reconverge (#1235). Not pure, not testable, not ours. |
| **Expansion state — split** | which rows are open | `documentState.collapsedNodes`/`pinnedNodes` (reducer-backed) **+** `ToolBlock.postCompletionHold`, `UserMessageBlock.hovering`, `MarkdownBlock.expanded` (component-local signals) | The thing that *determines* a row's height is **partly invisible to the reducer**. Height can change with no reducer action. |
| **Zoom** | render scale | CSS `zoom` on `.agent-view` **+** scattered `÷ zoomFactor` in `measureElement` | A units layer threaded through measurement by hand; easy to double-count (the original #1231 bug). |

Because height isn't deterministic state, the system **can't know a row's target
height synchronously** when expansion changes — it mutates the DOM and *waits for
a racy ResizeObserver* to tell it what happened. That wait is the race. Patching
it locally (data-index sync, content-visibility, undefined guards) has bought
correctness on the happy path but the **drift under churn (#1235) is unfixable
locally** because the cache that drifts isn't ours.

This spec proposes the structural fix the symptoms keep pointing at: **one
per-pane reducer slice that owns row heights, expansion, and zoom as pure
deterministic state, with rendered positions as a pure projection.**

---

## 2. The invariant we want (and why a reducer gives it)

Today positions come out of TanStack's incrementally-mutated cache, which can be
internally inconsistent (a row's `size` and the next row's `start` disagree
during a reflow) → **overlap**.

If instead **position is a pure prefix-sum of a height map**:

```
start[0]      = scrollMargin
start[i+1]    = start[i] + height[i]          // by construction
end[i]        = start[i] + height[i] = start[i+1]
```

then **slots never overlap** — `start[i+1] === end[i]` always. The *only* way a
visual overlap can occur is if a row's **rendered** height exceeds the `height[i]`
used for layout. So the entire overlap bug class collapses to maintaining a single
invariant:

> **INV-1:** `height[i]` (the value layout uses) equals row *i*'s actual rendered
> height *in its current expansion state*.

A stale `height[i]` then degrades to at worst a **gap or a one-frame jump on
correction** — never an overlap, never the stuck −20.9px we saw. The reducer's
whole job becomes *keeping INV-1 true and converging it deterministically.* That
is tractable, pure, and testable; an imperative ResizeObserver cache is none of
those.

Two corollaries shape the design:

- **INV-2 (zoom-invariance):** all heights/positions are stored in **unzoomed CSS
  px**. Zoom is applied exactly once, by the single ancestor CSS `zoom`. A zoom
  change requires **zero** height recompute (formalizes the #1233 finding). Zoom
  enters the reducer only as the divisor at the *measurement-ingest boundary*.
- **INV-3 (state-keyed measurement):** a measured height is cached **per
  `(nodeId, expansionState)`**, never as a single scalar. The #1234 `0`/`784`
  garbage was a measurement taken in one expansion state contaminating another.
  Keying by state makes an expand/collapse toggle resolve to a *known target
  height immediately* (the cached value for the new state, or an estimate),
  synchronously — no measurement round-trip, no race.

---

## 3. Proposed slice: `agent-pane-layout` (Slice #11)

Per-pane (keyed by `blockId`), following the established slice triplet
(`types.ts` + `reducer.ts` pure core, `agent-pane-layout-store.ts` dispatch layer
with projections + `recordDispatch`). Distinct from the existing
`agent-pane-state` slice (turn/lifecycle/tokens) and from the roadmap's
`layout-reducer` #5 (window/focus/magnify) — this one owns **document-row
layout**.

### 3.1 State

```ts
// agent-pane-layout/types.ts

// In-flow FOOTPRINT state — the only thing that affects the prefix-sum.
// Resolved (OQ-2): hover "overlay" peek is presentational — it renders as
// an ABSOLUTE layer while the collapsed summary stays in flow, so it does
// NOT change a row's in-flow height and is NOT a layout input. Layout is
// therefore binary.
export type ExpansionState = "collapsed" | "expanded";

// WHY a row is open. Pin (user) outlives auto (running / pending / 3s
// post-completion hold): a hold-expiry collapses an "auto" row but is a
// no-op on a "pin" row. Mirrors today's `pinnedNodes` ∪ `autoExpanded()`.
export type Expansion =
    | { open: false }
    | { open: true; via: "pin" | "auto" };

export const inFlowState = (e: Expansion | undefined): ExpansionState =>
    e?.open ? "expanded" : "collapsed";

export interface RowHeight {
    // measured CSS-px height per in-flow state (INV-3). undefined = unmeasured.
    collapsed?: number;
    expanded?: number;
}

export interface AgentPaneLayoutState {
    zoom: number;                                 // INV-2: layout is zoom-INVARIANT; this only scales render
    orderedIds: ReadonlyArray<string>;            // FULL virtualized-region node ids, in order (not just the window)
    idSet: ReadonlySet<string>;                   // O(1) membership over orderedIds — drops late measures for removed ids
    expansion: ReadonlyMap<string, Expansion>;    // unified — absorbs collapsed/pinned/auto-hold/cancel-thinking
    heights: ReadonlyMap<string, RowHeight>;      // measured, per state, unzoomed CSS px
    estimates: ReadonlyMap<string, RowHeight>;    // estimator output, per state (fallback when unmeasured)
    scrollTop: number;                            // unzoomed CSS px (windowing input)
    viewportPx: number;                           // scroll container clientHeight / zoom
    scrollMarginPx: number;                       // header offset above the virtualized region
    overscan: number;
}
```

Because `orderedIds` is the **full** virtualized set (not the visible window),
a row that merely scrolls out of view stays in `orderedIds`, so its measured
heights survive — fixing the stuck-estimate-after-recycle case structurally
(heights are keyed by stable `nodeId`, never by a recycled DOM element).

### 3.2 Commands (the only ways layout changes)

```ts
export type AgentPaneLayoutCommand =
    // ── document shape ──────────────────────────────────────────────
    | { type: "NodesChanged"; orderedIds: ReadonlyArray<string> }   // append/prepend/truncate → prune ids ∉ new set
    // ── expansion (unified; replaces the scattered signals) ─────────
    | { type: "UserExpanded"; nodeId: string }                      // pin open  → {open:true, via:"pin"}
    | { type: "UserCollapsed"; nodeId: string }                     // unpin     → {open:false}
    | { type: "AutoExpandStarted"; nodeId: string }                 // running/pending → {open:true, via:"auto"} (pin wins)
    | { type: "AutoExpandHoldExpired"; nodeId: string }             // 3s timer fired (store side-effect) → collapse IF via:"auto"
    // ── measurement ingest (normalized ÷zoom at the boundary) ───────
    | { type: "RowMeasured"; nodeId: string; state: ExpansionState; cssPx: number }   // state ∈ {collapsed, expanded}
    | { type: "EstimateSet"; nodeId: string; state: ExpansionState; cssPx: number }
    | { type: "MeasurementInvalidated"; nodeId: string }            // content streamed/grew → clears the WHOLE RowHeight
    // ── viewport / zoom ─────────────────────────────────────────────
    | { type: "Scrolled"; scrollTop: number; viewportPx: number }
    | { type: "ScrollMarginChanged"; px: number }
    | { type: "ZoomChanged"; zoom: number };                        // NO height recompute (INV-2)
```

The two **timers** (3 s post-completion hold; user-message hover delay) live in
the store layer as side effects that *dispatch* commands — never in the reducer.
The store arms the hold timer when a tool transitions to a terminal status and
dispatches `AutoExpandHoldExpired` on fire; the reducer collapses the row only if
it is still `via:"auto"` (a user pin during the hold makes it `via:"pin"` and the
expiry becomes a no-op).

### 3.3 Events (audit ring; surface in the Ctrl+Shift+D diag panel)

```ts
export type AgentPaneLayoutEvent =
    | { type: "row-measured"; nodeId: string; state: ExpansionState; delta: number }
    | { type: "expansion-changed"; nodeId: string; from: Expansion; to: Expansion } // Expansion = {open:false}|{open:true;via:"pin"|"auto"} — carries `via` so consumers can tell a user-pin from an auto-hold
    | { type: "measurement-invalidated"; nodeId: string }
    | { type: "zoom-changed-no-relayout"; zoom: number }   // explicit: proves INV-2 held
    | { type: "ids-pruned"; removed: number }
    | { type: "command-dropped"; reason: string };         // per conventions §"suppression events"
```

### 3.4 Selectors (pure projections — the renderer reads these)

```ts
// effective in-flow height of a row, current state, unzoomed CSS px.
// (Matches the shipped impl: no "overlay" branch — overlay is presentational,
// not a layout state, per OQ-2/§3.1/§5. state = inFlowState(expansion.get(id)).)
effectiveHeight(s, id): number =
    s.heights.get(id)?.[state] ?? s.estimates.get(id)?.[state] ?? DEFAULT_ROW_PX

// prefix-sum positions — THE guarantee (INV-1). O(n); see §7 perf note.
positions(s): Array<{ id; start; height }>   // start[i+1] = start[i] + height[i]

totalSize(s): number                          // start[last] + height[last] - scrollMargin

// windowing: first/last visible index via binary search over prefix sums + overscan
window(s): { startIndex; endIndex }
```

### 3.5 Reducer shape (pure)

`update(state, command): { state, events }` — same signature as every other slice.
Highlights:

- **`ZoomChanged`** sets `state.zoom` and emits `zoom-changed-no-relayout`. It does
  **not** touch `heights`/`estimates` (INV-2). The renderer's single ancestor
  `zoom` re-scales everything; positions are unzoomed and unchanged.
- **`RowMeasured`** writes `heights[nodeId][state] = cssPx` (INV-3). If it differs
  from the prior layout height, that's the *only* thing that shifts subsequent
  positions — deterministically, via the prefix-sum selector, all at once (never
  a partial cache update).
- **`UserExpanded/UserCollapsed`/`AutoExpand*`** change `expansion[nodeId]`; the projected height
  immediately switches to the cached measurement for the new state (or estimate).
  **No DOM round-trip needed to know the target height** — kills the expand race.
- **`NodesChanged`** replaces `orderedIds` and **prunes** `expansion`/`heights`/
  `estimates` for removed ids while **preserving surviving ids** (measurements
  survive scroll-recycle — fixes the stuck-estimate-after-churn, because a row's
  height is keyed by stable nodeId, not by a recycled DOM element / data-index).

### 3.6 Store / dispatch layer

Mirrors `browser-pane-state-store.ts`: `Map<blockId, Slot{state, proj}>`,
`registerPane`/`unregisterPane`/`dispatch(blockId, cmd, source)`, project only
changed fields to Solid signals, `recordDispatch({slice:"agent-pane-layout", ...})`.
The two **timers** (post-completion hold; user-message hover-expand delay) live
here as side effects that *dispatch* `AutoExpandHoldExpired` / `UserExpanded/UserCollapsed` — the
reducer stays pure (cf. the `postCompletionHold` self-loop bug already documented
in `ToolBlock.tsx`).

---

## 4. Renderer becomes a pure projection

`AgentDocumentVirtualList` stops asking TanStack "what are the positions?" and
instead **renders the slice's `positions()`/`window()`**:

```
slice.window()        → which ids to mount
slice.positions()     → translateY(start) per mounted row (unzoomed CSS px)
slice.totalSize()     → spacer height
```

- On a row's `ResizeObserver`/ref fire → `dispatch(RowMeasured{nodeId, state, gbcr.height / zoom})`.
  (The ÷zoom normalization lives at this single boundary — INV-2.)
- On expand/collapse/pin interaction → `dispatch(UserExpanded/UserCollapsed{...})`.
- On scroll → `dispatch(Scrolled{...})`; on zoom meta change → `dispatch(ZoomChanged)`.

### 4.1 Relationship to TanStack `@tanstack/virtual-core`

Two viable end-states; spec recommends **(B)** as the target, reached via the
phases in §6:

- **(A) Feed TanStack from the slice.** Keep TanStack windowing; make
  `estimateSize(i)` return `effectiveHeight(slice, id)` (so it's the cached
  measurement when known, never a bad guess) and `measureElement` dispatch
  `RowMeasured` and return the slice's authoritative height. *Lower risk, but
  TanStack still owns the position prefix-sum → INV-1 not fully guaranteed.*
- **(B) Slice owns positions; TanStack retired from the agent pane.** Positions
  come from the slice's prefix-sum; a ~80-line pure windowing selector (binary
  search over prefix sums + overscan) replaces `getVirtualItems()`. *This is what
  makes overlap structurally impossible (§2). It also deletes the `data-index`
  ref race, the `getVirtualItems()`-undefined crash guard, and the
  `shouldMeasureDuringScroll` gating — all artifacts of not owning the cache.*

The **streaming buffer** (trailing nodes in normal flow, no `translateY`) is **out
of scope** — it has no position math to get wrong. The slice models only the
virtualized region (`partition().virtualizedNodes`).

---

## 5. Unifying the scattered expansion state

Today a row's "is it open" is computed from up to four places. The slice makes
`expansion: Map<nodeId, ExpansionState>` the **single** authority; components
become thin:

| Today (ad-hoc) | file | Becomes |
|---|---|---|
| `documentState.collapsedNodes` / `pinnedNodes` | `types.ts`, `AgentDocumentView` toggles | dispatch `UserExpanded/UserCollapsed`; selector reads `expansion` |
| `ToolBlock.postCompletionHold` (3s timer) + `autoExpanded()` | `ToolBlock.tsx` | store-layer timer → `AutoExpandStarted` / `AutoExpandHoldExpired`; `expanded()` reads slice |
| `MarkdownBlock.expanded()` (canceled-thinking) | `MarkdownBlock.tsx` | `UserExpanded` / `UserCollapsed` on click |
| `UserMessageBlock.hovering` → `bodyMode "overlay"` | `UserMessageBlock.tsx` | **stays component-local** — it's presentational (absolute peek; summary stays in flow) and does **not** change in-flow height, so it is **not** a layout input (resolved OQ-2). |

Net: the reducer sees every input that changes a row's **in-flow height**, so
every such change is preceded by a command — the precondition for INV-1 and for
deterministic tests. (It also lets the estimator be a pure function of slice state
instead of reading a separate `documentState()`.) The hover-peek overlay is
deliberately excluded: it has no in-flow footprint, so pulling it into the layout
reducer would add state with no layout meaning.

---

## 6. Migration phases (each independently shippable + verifiable)

- **Phase 0 — shadow.** Add the slice (state+reducer+store) and dispatch into it
  from the existing render path, but **keep rendering from TanStack.** Log
  divergence between slice `positions()` and TanStack measurements. Goal: validate
  the model against live traffic with zero user-visible change. Ship behind the
  existing dev-only instrumentation.
- **Phase 1 — unify expansion (§5).** Move `collapsed/pinned/hold/hover/cancel`
  into the slice; components dispatch `UserExpanded/UserCollapsed`. **This alone removes the
  component-local desync** and makes expansion testable. Still TanStack-measured.
- **Phase 2 — route measurement (4.1-A).** `estimateSize`/`measureElement` read/
  dispatch through the slice; measurements keyed by `(nodeId, state)` (INV-3).
  Kills the `0`/`784` cross-state contamination.
- **Phase 3 — slice owns positions (4.1-B).** Replace `getVirtualItems()` with the
  prefix-sum windowing selector. **Overlap becomes structurally impossible
  (INV-1).** Retire TanStack from the agent pane; delete the data-index ref race,
  the undefined-virtualItem guard, and `shouldMeasureDuringScroll` workarounds.
- **Phase 4 — formalize zoom (INV-2).** Single ÷zoom at the `RowMeasured` boundary;
  remove any other zoom reads from the layout path; assert `zoom-changed-no-relayout`.

Phases 1–3 each close a distinct bug class from §1; Phase 3 is the one that closes
#1235.

---

## 7. Testing

The whole point of moving to a reducer is that correctness becomes **provable**,
per the project's state-machine testing discipline:

- **Property test — non-overlap (INV-1).** For *any* sequence of commands
  (random `NodesChanged`/`UserExpanded/UserCollapsed`/`RowMeasured`/`Scrolled`/`ZoomChanged`),
  assert `positions()` is monotonic: `start[i+1] === end[i]` for all i. This is the
  invariant #1235 violates; here it holds by construction and is *tested* to stay
  so under churn. (Anti-vacuity: assert the generated sequence actually produced
  ≥1 measurement that changed a height.)
- **State cross-product.** `expansion ∈ {collapsed, expanded}` ×
  `measured? ∈ {yes,no}` × `zoom ∈ {0.5,1,2}` × `churn ∈ {toggle, stream-grow,
  prune}` — table-drive the reducer; assert effective height + position for each
  cell. Build this table *before* coding (cf. the orchestrator-test lesson from
  PR #702).
- **Replay production emit order.** Feed the exact command order the live render
  path emits (measure-after-paint, expansion-before-measure) — out-of-order tests
  give false confidence.
- **Zoom-invariance.** `ZoomChanged` must not alter any `positions()` value; only
  the render multiplier. Assert `heights`/`estimates` referentially unchanged.
- **Live CDP verification of record** (jsdom can't do CSS `zoom`/layout). Reuse the
  recipe from this cycle: read each `.agent-document-row` `translateY` vs
  `offsetHeight`, and rendered-top overlap; run the **fresh-reload → N
  expand/collapse cycles** churn test from #1235 and assert **0 drift** (the
  current code accumulates 5 mismatches / 1 overlap after 9 cycles).

---

## 8. Why this kills each bug we chased

| Bug | Root | Eliminated by |
|---|---|---|
| Overlap under zoom (#1231) | measure (zoomed px) vs estimate (css px) unit clash | INV-2: one unit, zoom applied once at render |
| Stuck-at-estimate rows (#1233) | data-index null when measureElement ref fired → never observed | Phase 3: no data-index/ResizeObserver dance — height is reducer state keyed by nodeId |
| Expand crash `(reading 'index')` (#1234) | `getVirtualItems()` transiently undefined mid-reflow | Phase 3: no `getVirtualItems()`; window is a pure selector |
| Collapsed-overlay layout cost (#1234) | hidden content laid out | orthogonal (CSS `content-visibility`, already shipped) — slice is compatible |
| Drift under churn (#1235) | TanStack incremental cache caches inconsistent sizes that don't reconverge; affects non-tool rows too | INV-1 + INV-3: positions = prefix-sum of state-keyed heights; deterministic reconvergence on every `RowMeasured` |

---

## 9. Risks & non-goals

- **Scope.** This is a multi-PR migration, not a single change. Phase 0/1 deliver
  value (unified, testable expansion) with low risk; the structural guarantee
  needs Phase 3. Do **not** attempt all at once — each phase ships and is verified
  live before the next (cf. "stop at perfect", the #1177 lesson).
- **Perf of prefix-sum.** `positions()` is O(n) over the virtualized region. For
  very long histories recompute only on height/expansion/ids change (memoized),
  and if profiling shows it hot, back `heights` with a **Fenwick/segment tree** for
  O(log n) position queries + O(log n) updates. Measure first (the perf HUD +
  the §7 CDP recipe); don't pre-optimize.
- **Behavior parity.** Phase 1 must reproduce today's exact expansion semantics
  (startup-collapse, 3s hold, hover-peek overlay, canceled-thinking) — capture
  them as the cross-product table before refactoring.
- **Non-goals:** streaming buffer layout (normal flow, no bug); window/tab layout
  (`layout-reducer` #5); the `content-visibility` perf fix (#1234, independent).

---

## 10. Files (Phase 1 footprint; later phases extend)

| File | Change |
|---|---|
| `frontend/app/store/agent-pane-layout/types.ts` | new — state, commands, events, `initialState` |
| `frontend/app/store/agent-pane-layout/reducer.ts` | new — pure `update` + selectors |
| `frontend/app/store/agent-pane-layout-store.ts` | new — dispatch layer, projections, timers, `recordDispatch` |
| `frontend/app/store/agent-pane-layout/reducer.test.ts` | new — property + cross-product tests (§7) |
| `frontend/app/view/agent/components/ToolBlock.tsx` | read expansion from slice; dispatch on pin; store-layer hold timer |
| `frontend/app/view/agent/components/UserMessageBlock.tsx`, `MarkdownBlock.tsx`, `AgentDocumentView.tsx` | dispatch `UserExpanded/UserCollapsed`; drop local expansion signals |
| `frontend/app/view/agent/virtualization/AgentDocumentVirtualList.tsx` | (Ph2/3) read positions/window from slice; route measurement |
| `frontend/app/view/agent/virtualization/renderers.ts` | estimators become `EstimateSet` producers keyed by state |
| `frontend/app/devtools/diag-panel.tsx` | surfaces the new slice automatically (audit ring) |

---

## 11. Resolved decisions (were open questions)

1. **Phase 3 windowing — BUILD, don't fork.** A pure `windowRange(state)` selector:
   prefix-sum the in-flow heights, binary-search the first row whose `end >
   scrollTop` and the last whose `start < scrollTop + viewportPx`, pad by
   `overscan`. ~40 lines, fully unit-testable, no imperative cache. Forking
   `virtual-core` would re-import the very mutable cache we're removing.
2. **Hover-peek "overlay" is NOT a layout state (see §3.1, §5).** It renders as an
   absolute layer with the collapsed summary still in flow, so it has zero in-flow
   footprint and stays a component-local presentational signal. Layout state is
   binary `collapsed | expanded`. This both simplifies the model and avoids
   anchor-math coupling (anchors are in-flow nodes; overlay rows have normal
   collapsed extent).
3. **Pruning by set membership — no `reason` needed.** `orderedIds` is the FULL
   virtualized region, so scroll-out never removes an id; `NodesChanged` prunes
   exactly the ids absent from the new set and preserves heights for survivors.
   Re-entry (scroll back) reuses the surviving measured height for its state.
   Content growth is the only invalidation path: `MeasurementInvalidated{nodeId}`
   clears that row's WHOLE `RowHeight` (both states) to force a clean re-measure.
4. **Decouple from `agent-pane-state` (#4) — translate at the edge.** The layout
   slice does NOT import or subscribe to the turn/tool-status slice. The component
   (or a thin adapter `createEffect` watching `node.status`) translates
   running/pending → `AutoExpandStarted` and a terminal transition → arm the hold
   timer → `AutoExpandHoldExpired`. Slices stay independent; the audit ring shows
   the translated command with its `source`.

### Phasing note
Phase 0 (this implementation) ships the pure slice + store + tests — zero
render-path wiring, zero behaviour change, fully covered by the §7 property and
cross-product tests. Phases 1–4 (wiring, per §6) follow as separate verified PRs.
