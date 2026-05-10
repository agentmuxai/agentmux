# Agent pane virtualization & intelligent perf probing — redesign

**Status:** Proposed
**Owner:** AgentA
**Date:** 2026-05-09
**Replaces:** PR #773 (`@tanstack/solid-virtual` retrofit, reverted)
**Related:** Issue #774 (tab content reveal gate), Issue #769 (new-window bootstrap)

## Driving observation

PR #773 added virtualization to `AgentDocumentView` by wrapping the existing flex-flow `<For>` with TanStack's virtualizer. The benchmarks confirmed the win is real (long-task storms during tab-switch dropped substantially), but interactive use surfaced regressions in three areas:

| Symptom | Root cause |
|---|---|
| Visible gaps between short messages | `estimateSize` dominates layout until `measureElement` settles; per-kind heights vary too much for one estimate |
| Markdown tables break | Tables lose layout context inside absolute-positioned rows — no width to constrain against, default `table-layout: auto` overflows |
| Long thinking blocks show only first word | Wrapper height doesn't grow chunk-by-chunk during streaming; ResizeObserver lag during token-stream + virtualizer keying |
| Pagination scroll-restore fragile | Pixel-math worked pre-virt but not with measured virtual heights; needed bespoke anchor rewrite |
| Jump-to-node DOM query fails | `querySelector` finds nothing when target is outside the virtual window; needed `scrollToIndex` first + measure |

Each fix-as-we-go workaround introduced its own edges. The deeper issue is that the original component assumed every node was in the DOM, and virtualization is the wrong constraint to retrofit.

This spec treats virtualization as a first-class architectural concern, designed alongside an integrated perf-probing layer that makes regressions visible during development.

## Goals

1. **Bounded DOM size** — only ~50 nodes mounted regardless of session length. Layout-tree memory and reflow cost stay constant in node count.
2. **Correct rendering of all content kinds** — markdown tables, code diffs, tool blocks, streaming thinking, subagent links. No more chasing per-kind regressions.
3. **Predictable streaming behavior** — content arriving chunk-by-chunk doesn't stutter, jump, or clip.
4. **Robust scroll preservation** — scroll position survives prepend (history pagination), append (new turn), and re-mount (tab switch).
5. **Integrated observability** — per-kind perf marks built into the render contract; dev HUD shows p50/p95 per kind so estimate misses and regressions are visible immediately, not after user reports.

## Non-goals

- **Server-side rendering / hydration.** Out of scope; AgentMux is a desktop app, not SSR.
- **Re-architecting the agent state machine / reducer.** Pure-view-layer change. The reducer (truncate-grace, dedup, session phase) is unchanged.
- **Replacing `@tanstack/solid-virtual`.** It's still the right primitive; the redesign is in how we use it.
- **Generalized virtualization framework** for other panes (terminal, browser). Each has different constraints.

## Industry research summary

Mature chat applications fall into two camps:

**Virtualizing camp** — Slack-clones built on Stream.io's `VirtualizedMessageList` (which wraps `react-virtuoso`). Patterns: stable per-item keys, per-kind size estimators, anchor-based scroll preservation, `ResizeObserver`-driven measurement, explicit "stick to bottom" state separate from scroll position.

**Non-virtualizing camp** — ChatGPT, Claude.ai, Cursor, Notion. They cap conversation length and rely on `content-visibility: auto` plus block-level memoization. At < ~500 nodes this beats virtualization for rich content because the browser's compositor optimizations kick in without the bookkeeping cost.

Our target session size (2000+ nodes) puts us firmly in the virtualizing camp, but with a critical hybrid: **the last N nodes (streaming zone) stay unvirtualized** so the streaming-cut-off class of bug is impossible by construction. This matches what `react-virtuoso`'s Message List does internally, plus what `use-stick-to-bottom` (StackBlitz, powers Bolt.new and v0-style tools) implements for the tail.

**Anchor patterns**: production systems maintain two anchors — a head anchor `{nodeId, offsetPx}` for pagination, and a tail flag `stickToBottom: bool` for streaming. Source-of-truth is data, not DOM. CSS `overflow-anchor: auto` is a belt-and-suspenders complement that works in CEF/Chromium.

**Tables**: canonical fix is `width: 100%` on the row plus `table-layout: fixed` on every markdown table. The first row determines column widths; eliminates reflow-during-stream.

**References:**
- [react-virtuoso Message List](https://virtuoso.dev/virtuoso-message-list/)
- [Stream VirtualizedMessageList](https://getstream.io/chat/docs/sdk/react/components/core-components/virtualized_list/)
- [stackblitz-labs/use-stick-to-bottom](https://github.com/stackblitz-labs/use-stick-to-bottom)
- [Discord Mobile perf rewrite](https://discord.com/blog/supercharging-discord-mobile-our-journey-to-a-faster-app)
- [react-virtualized Table.md](https://github.com/bvaughn/react-virtualized/blob/master/docs/Table.md)
- [overflow-anchor MDN](https://developer.mozilla.org/en-US/docs/Web/CSS/overflow-anchor)
- [3perf React monitoring](https://3perf.com/blog/react-monitoring/)

## Architecture

### Data model

Document nodes are an **immutable indexed list keyed by stable `nodeId`**. The reducer already produces this. The view layer reads it via SolidJS signals. Two derived structures live in a Solid store:

```ts
interface AgentViewState {
    // Indexed access for virtualizer, jump-to-node, anchor lookup.
    nodes: DocumentNode[];
    nodeIndex: Map<string, number>;  // O(1) id → index lookup

    // Scroll state lives here — NOT in DOM scrollTop.
    headAnchor: { nodeId: string; offsetPx: number } | null;
    stickToBottom: boolean;

    // Streaming state.
    streamingNodeId: string | null;  // pinned out of virtualization range
    streamingTokenBatch: Map<string, string[]>;  // RAF-batched token deltas
}
```

**Key invariant:** all scroll/render decisions read from this store. The DOM is a projection.

### Render contract

Each `DocumentNode` kind declares:

```ts
interface NodeKindRenderer<K extends NodeKind> {
    component: Component<{ node: NodeOf<K> }>;
    estimatedSize: (node: NodeOf<K>, state: DocumentState) => number;
    isStreamingCapable: boolean;
    /** Optional: render in a dedicated unvirtualized layer (e.g., always-visible auth box). */
    pinnedLayer?: 'top' | 'bottom';
}
```

Per-kind estimators replace the single `estimateSize: () => 80`:

| Kind | Estimator |
|---|---|
| `agent_message` | `Math.ceil(text.length / 80) * 24` (text-line heuristic), capped at 320 |
| `user_message` | Same heuristic; collapsed → 32, expanded → estimator |
| `tool` | Pinned (expanded) → 200, collapsed → 32 |
| `agent_section` | 48 |
| `markdown` | `Math.ceil(text.length / 80) * 24` |
| `subagent_link` | 56 (fixed) |

Estimators are validated against actual measurements via the perf probe (§ Intelligent perf probing). When p50 measured size diverges from the estimator by > 30%, the dev HUD flags it.

### Scroll state in data

`headAnchor` and `stickToBottom` live in the store. Mutation rules:

- **User scrolls up** — `stickToBottom = false`. If scroll near top (< 50px), trigger pagination; before prepend, capture `headAnchor = { nodeId, offsetPx }` of topmost visible node.
- **User scrolls to bottom** — `stickToBottom = true`. `headAnchor = null`.
- **New tail node arrives** — if `stickToBottom`, emit scrollToIndex(last). Else: no-op (item appends silently below viewport).
- **Pagination prepends** — after prepend, scroll to `getOffsetForId(headAnchor.nodeId) - headAnchor.offsetPx`. Restore anchor.
- **Tab re-mount** — restore from store: if `stickToBottom`, scroll to last; else scroll to `headAnchor`.

This eliminates the entire class of "scroll position is wrong after X" bugs because there's one source of truth.

### Hybrid virtualization

```
┌─ scroll container (.agent-document) ────────┐
│  ┌─ virtualized region ────────────────────┐│
│  │  [items 0 .. N-K-1]                     ││  ← off-screen items NOT mounted
│  │  position: relative                     ││
│  │  height = sum of estimated/measured     ││
│  └─────────────────────────────────────────┘│
│                                             │
│  ┌─ streaming buffer ──────────────────────┐│  ← always mounted
│  │  [items N-K .. N-1]                     ││  ← K = STREAMING_BUFFER_SIZE
│  │  normal flex flow, no virtualization    ││  ← typically last 50 nodes
│  └─────────────────────────────────────────┘│
└─────────────────────────────────────────────┘
```

The streaming buffer is a regular `<For>` over the tail slice. New tokens flow into nodes here without virtualizer measurement lag. When a node ages past `STREAMING_BUFFER_SIZE`, it transitions into the virtualized region — at which point its measurement is stable (streaming complete).

This single design decision eliminates the streaming-cut-off bug class entirely.

### Tables and rich content

Two CSS rules cover the table-distortion class:

```scss
.agent-document-virtualizer .agent-document-row {
    width: 100%;  // Anchor for any width-relative children.
}

.agent-document-row :is(table, .markdown-table) {
    width: 100%;
    table-layout: fixed;  // First row determines column widths.
}
```

For tool blocks with diff viewers / bash output:
- Collapsed by default (matches existing UX).
- Inner content lazy-mounted via `<Show when={isExpanded()}>` not CSS `display: none` — so the layout cost only appears on expand.
- Expanded estimator returns 320 (initial), settles via measurement.

### Anchor — CSS + JS belt-and-suspenders

```scss
.agent-document {
    overflow-anchor: auto;  // Browser auto-anchor for unmounted img/font loads.
}
```

CEF is Chromium so this works. The JS anchor handles cases CSS can't (scroll restoration across re-mount, programmatic `scrollToIndex`).

## Intelligent perf probing

Perf probing is built into the render contract, not an afterthought. Three layers:

### Layer 1: Per-kind render marks

Every row mount fires:

```ts
const nodeId = node.id;
const kind = node.type;
performance.mark(`agent-row:${kind}:${nodeId}:mount-start`);
// ... component renders ...
performance.mark(`agent-row:${kind}:${nodeId}:mount-end`);
performance.measure(`agent-row:${kind}`, `agent-row:${kind}:${nodeId}:mount-start`, `agent-row:${kind}:${nodeId}:mount-end`);
```

A single `PerformanceObserver({ entryTypes: ['measure'] })` aggregates per-kind p50 / p95 / max into a Solid store. Bounded buffer (last 100 measurements per kind).

### Layer 2: Estimator validation

After `measureElement` settles a row, compare actual vs estimated:

```ts
const actual = measuredSize;
const estimated = renderer.estimatedSize(node, state);
const errorPct = Math.abs(actual - estimated) / estimated;
if (errorPct > 0.30) {
    perfStore.recordEstimatorMiss(node.type, estimated, actual);
}
```

The HUD surfaces estimator misses by kind. When a kind shows persistent miss, the estimator gets recalibrated.

### Layer 3: Layout-shift attribution

```ts
new PerformanceObserver(entries => {
    for (const e of entries.getEntries()) {
        const inAgentDoc = (e as LayoutShift).sources?.some(
            s => s.node?.closest?.('.agent-document')
        );
        if (inAgentDoc) {
            perfStore.recordLayoutShift(e.value, e.startTime);
        }
    }
}).observe({ type: 'layout-shift', buffered: true });
```

Any unexpected shift inside the agent pane is a measurement miss or estimator error. Caught at the source instead of via user reports.

### Dev HUD

A diagnostic panel (Ctrl+Shift+D extends the existing one from slice #9 phase 5):

```
Agent pane perf
─────────────────────────────────────────────
Render time per kind (last 100):
  agent_message    p50: 2.1ms  p95: 8.3ms  max: 23.0ms  n=87
  user_message     p50: 1.4ms  p95: 3.2ms  max:  6.1ms  n=12
  tool             p50: 5.6ms  p95: 18.4ms max: 41.0ms  n=34
  markdown         p50: 4.2ms  p95: 12.1ms max: 28.0ms  n=56
  subagent_link    p50: 0.8ms  p95: 1.2ms  max:  2.1ms  n=4

Estimator accuracy:
  agent_message    avg miss: 8%   ✓
  tool             avg miss: 47%  ⚠ recalibrate (under-estimating expanded)
  markdown         avg miss: 12%  ✓

Layout shifts in agent pane (last 60s): 3
  ↳ 23.4s ago: 0.012 (subagent_link mount)
  ↳ 41.2s ago: 0.008 (tool expand)
  ↳ 58.7s ago: 0.034 (markdown table — likely table-layout regression)

Streaming buffer:
  size: 50 / 50  oldest: 2m14s  newest: streaming…
  pinned node: msg_8a3f (active stream)
```

This makes regressions visible during development. A future PR adding a new node kind will see its estimator miss immediately if wrong.

### Production behavior

All probing is dev-only by default. Production builds ship with `markStart`/`markEnd` no-ops (already conditional in `frontend/perf/marks.ts`). Layout-shift observer disabled. HUD inaccessible. Zero runtime cost.

## Migration plan

**Phase 1 — Foundation** (~1.5 days):
- New `frontend/app/view/agent/virtualization/` directory.
- `AgentViewState` Solid store + reducer integration (no UI change yet).
- Per-kind renderer registry with explicit estimator functions.
- Tests for state transitions (anchor capture, stickToBottom flips, pagination restore math).

**Phase 2 — Virtualization layer** (~1.5 days):
- New `AgentDocumentVirtualList` component.
- Hybrid render: virtualized region + streaming buffer.
- CSS: `width: 100%` rows, `table-layout: fixed`, `overflow-anchor: auto`.
- Replace `AgentDocumentView`'s `<For>` block with `<AgentDocumentVirtualList>`.
- Verify: tables, code diffs, streaming, scroll position survives all flows.

**Phase 3 — Perf probing** (~1 day):
- Per-kind marks via render contract HOC.
- Layout-shift observer scoped to `.agent-document`.
- Extend diag panel with the agent pane section.
- Validate estimators against real session data; tune.

**Phase 4 — Production hardening** (~0.5 day):
- Confirm probing is no-op in prod build.
- Smoke test against a 2000+ node session.
- Tab-switch perf regression test (target: ≤ 32ms per switch into populated agent pane).

**Total: ~4.5 days.** Ships as a single PR or split across phases — author's call.

## Risks

| Risk | Mitigation |
|---|---|
| Streaming buffer size wrong (too small → scroll jank when items age out; too large → defeats virtualization) | Start with 50, instrument transition events, tune in Phase 4 |
| Per-kind estimators wrong for new content lengths | Estimator validation in HUD catches this immediately |
| `overflow-anchor` interferes with explicit anchor restore | Test interaction; if conflict, add `overflow-anchor: none` on virtualizer container during programmatic scroll |
| Solid store reactivity overhead | Validated by perf probing — render time per kind tells us if signal updates are too granular |
| Tables still misbehave after `table-layout: fixed` | If column-content discovery is required, fall back to passthrough portal pattern (Notion approach) |

## Out of scope (explicit)

- **Replacing `@tanstack/solid-virtual` with another virtualizer.** It's the right primitive.
- **Markdown rendering rewrite.** Keep current `MarkdownBlock`; just constrain its width.
- **Real-time perf telemetry to backend / external service.** All in-process, dev-mode.
- **Mobile/touch gestures.** Desktop only.
- **Conversation cap.** No "auto-archive after 1000 messages" — virtualization makes that unnecessary.

## Cross-references

- PR #773 (reverted) — this spec replaces it.
- Issue #774 — tab content reveal gate. Complementary; the gate hides content until stable, virtualization makes "stable" possible.
- `frontend/perf/marks.ts` — existing mark API; extend with `markRow(kind, id)` helper.
- `frontend/app/devtools/diag-panel.tsx` — existing diag panel from slice #9 phase 5; extend with the agent pane section.
- `frontend/app/view/agent/components/AgentDocumentView.tsx` — current implementation; will become a thin shell delegating to `AgentDocumentVirtualList`.

## Driving observation (verbatim)

> "do we need all that complexity? are we adding it because we put virtualization afterward? perhaps we need to rearchitrect the agent pane with virtualization from the start? the page is still distored, the tables are messed up"
