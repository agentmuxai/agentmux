# SPEC: Agent pane tab-switch perf

**Date:** 2026-05-27
**Author:** AgentA
**Status:** Design — perf investigation + multi-phase remediation plan.

---

## The problem

Switching between tabs that each host an agent pane is **noticeably slow** — long enough to feel like the UI froze briefly. Same instance, same tab strip, same window. The agent pane's conversation view (the `AgentDocumentVirtualList`) is the suspected dominant cost; this spec scopes that down and proposes a remediation plan, ordered by impact-per-LOC.

---

## What actually happens on tab switch today

### Tabs stay mounted

Per `frontend/app/workspace/workspace.tsx:46`, inactive tabs use `display: none` rather than unmounting. From the inline comment:

> Inactive tabs are `display: none` already.

So switching tabs is NOT a remount cycle — the prior tab's `AgentViewModel`, `useAgentStream` subscription, virtualizer, and document store all remain alive. The cost is in the **visibility-reveal pipeline**, not in cold-mount.

### Reveal gate already exists (issue #774)

`workspace.tsx:40-48` documents a known issue: when an active tab is "revealed" (display: none → block), a "piecemeal mount cascade" paints stage-by-stage. The team's mitigation is to apply `visibility: hidden` to the active tab during reveal so the cascade can settle off-screen, lifted by `frontend/app/tab/tab-reveal.ts`'s frame-budget detector.

The cascade itself isn't gone — the gate just hides its ugliness. The dominant cost still happens; we're paying for it but not seeing the partial paints.

### What runs in the cascade

1. **Virtualizer remeasure** — TanStack Virtual's `measureElement` reads `getBoundingClientRect()` on every rendered row. With `display: none` ancestors, those rects are 0/empty; on reveal, every row gets a fresh measurement pass. For 50 streaming-buffer rows + 5 overscan = ~55 sync `getBoundingClientRect` calls + layout invalidations.
2. **Markdown re-render** — `frontend/app/element/markdown.tsx` runs the full `unified` pipeline (`remarkParse` → `remarkRehype` → rehype-highlight → `toJsxRuntime`) **on every render**, with no memoization layer. Reactive reads inside the agent virtualizer can trigger fresh renders of every visible markdown node. Per-node cost is 2-10ms.
3. **Tool block panel hydration** — `ToolBlock` mounts its overlay panel components (`BashOutputViewer`, diff viewer, log tail). These are present in the DOM even when collapsed; they re-run their setup effects on reveal.
4. **IntersectionObserver / ResizeObserver fan-out** — every row registers with the observers; reveal triggers them all.
5. **Async snapshot restore (one-shot)** — for tabs visited the first time in a session, `useHistoryPagination` fires `AgentSessionReadCommand` RPC + dispatches `HistoryRestored`. Subsequent tab visits skip this.

### Existing perf instrumentation

We already log per-tab signals (`frontend/perf/observers.ts`):

| Tap | Threshold | Logged as |
|---|---|---|
| Long-task observer | ≥50ms | `console.warn("[perf] long-task ${duration}ms ...")` |
| IPC roundtrip | >16ms | `console.warn("[perf] ipc ${command} ${durationMs}ms")` |
| Partition recompute | per-call | `trail("agent:virt:partition", {...})` (render-trail, dumped on boundary catch) |
| Estimator measurement | per-row | `agentPerfStore.recordEstimatorMeasurement()` (dev only) |

So a baseline measurement is one `muxlog host '[perf]'` away — see §7 below.

---

## Best practices — what other systems do

### VS Code / Electron — keep renderers alive

VS Code [moved to sandboxed renderers in 2022](https://code.visualstudio.com/blogs/2022/11/28/vscode-sandbox) specifically so they could **keep renderer processes alive across navigations**:

> Traditionally the renderer process would terminate and restart every time a navigation occurs to another URL, which for VS Code meant that changing a workspace or reloading the window would recreate the renderer process, which can be slow.

Their general pattern: never throw away DOM / state you'll want back; gate visibility instead. AgentMux already follows this for tabs (display: none). The remaining wins are at the **per-pane visibility-reveal** layer, not at the tab-or-pane mount layer.

### SolidJS — `solid-keep-alive` for offscreen subtrees

[`solid-keep-alive`](https://github.com/JulianSoto/solid-keep-alive) provides a `KeepAlive` component that **takes ownership of children when their parent unmounts**, caching them with their signals/DOM intact. Useful in patterns where Solid's `<Show>` or `<Switch>` would otherwise discard a subtree. Less applicable for AgentMux since we don't unmount via `<Show>` — but the pattern (cache-on-unmount instead of dispose) is worth knowing for future split-pane cases.

### TanStack Virtual — `measureElement` cache + size accuracy

[TanStack docs](https://tanstack.com/virtual/latest/docs/api/virtualizer) and [LogRocket's guide](https://blog.logrocket.com/speed-up-long-lists-tanstack-virtual/) emphasize:

> The measurement cache is consumed only once, on the first `getMeasurements()` call after mount. — TanStack docs

> One common mistake is underestimating the importance of accurately estimating row or item sizes; an incorrect estimate can lead to janky scrolling behavior or items not being rendered when they should be. — LogRocket

For us, this means: **the more accurate our `estimateSize` predictions are at reveal time, the less work the post-reveal measurement cascade has to do.** We already log estimator accuracy in `agentPerfStore.recordEstimatorMeasurement`; if the dev HUD shows high miss rates per node-kind, we should tune the kind-specific defaults.

### Markdown — memoize aggressively

The [StudyRaid React-Markdown perf guide](https://app.studyraid.com/en/read/11460/359228/optimizing-large-markdown-documents) and the [Shiki best-performance guide](https://shiki.style/guide/best-performance) both call out the same thing:

> Memoizing your code renderer with `React.memo` prevents repeated renders from redoing expensive parsing operations.

> Implementing a caching system for Shiki Highlighter instances made build times 80% faster, and this was more effective than other suggested solutions. — [Caching Shiki for Faster Build Times](https://dev.to/iamhectorsosa/caching-shiki-for-faster-build-times-4llb)

Our `Markdown` component does NOT memoize — every render re-runs the full pipeline. For a finalized markdown node (`content` no longer changing), this is pure waste.

### Incremental + virtualized rendering

> For large Markdown documents, consider using virtualized rendering to only render the visible portion of the document. — [StudyRaid: Optimizing large Markdown documents](https://app.studyraid.com/en/read/11460/359228/optimizing-large-markdown-documents)

Already done at the document level (the virtualizer). The remaining target is the **inside of each markdown node** — if a single agent response is many paragraphs of markdown, the unified pipeline does it as one job. Splitting on `\n\n` boundaries and lazy-rendering paragraphs is a much bigger refactor (out of scope for this spec).

---

## Proposed approach

Three phases, each independently shippable and each tied to a measurement before/after.

### Phase 0 — Measure (no code change)

Required prerequisite. Without baseline numbers we can't tell which phase actually moved the needle.

**Method:**
1. User opens 2 tabs each with an agent pane (one ~50 nodes, one ~200 nodes).
2. Wakes both panes (send a message to each so the document is populated).
3. Switches tabs 5x in each direction.
4. We grep `[perf] long-task` and the `tab-reveal` log lines from `muxlog host` between the switch timestamps.

**Output:** a table per tab-switch direction:

| Direction | Long-tasks (count × avg ms) | Tab-reveal duration | Likely dominant cost |
|---|---|---|---|

If a single long-task dominates (e.g. 600ms of one frame), we know it's a sync hotspot. If many small tasks distribute the cost, it's a fan-out problem (observers, re-renders, layout cascade).

### Phase 1 — Markdown memoization (≈30 LOC + tests)

**Hypothesis (highest expected ROI):** every visible markdown node's content is being re-parsed on tab reveal because there's no cache.

**Change:** wrap the `Markdown` component with a memo by `(content, options)` key. When a node's content is unchanged between renders (the common case once a turn finishes), reuse the cached `toJsxRuntime` output.

Two implementations to consider:

**Option A — module-level WeakMap cache.** Keyed by `content` string identity. The unified pipeline output is JSX; we return the same cached fragment. Cleared automatically as old content strings get GC'd.

**Option B — Solid `createMemo` on `props.text`.** Per-component memo. Recomputes only when `props.text` changes referentially. Simpler, doesn't risk cross-pane leaks.

**Preference: B.** Local memos compose better with Solid's reactivity, and the streaming buffer's per-token updates would invalidate a content-keyed module-level cache anyway. Won't help streaming, but **will** help the dominant case (revisiting a tab whose content is finalized).

**Verification:** Phase 0 measurement repeats. Long-task count for the second-tab-visit case should drop substantially.

### Phase 2 — Defer expensive renderers until in-viewport (≈80 LOC)

Tool blocks (`ToolBlock`) mount their bodies (overlays, log tails, diff viewers) eagerly even when collapsed. Markdown nodes with code blocks invoke syntax highlighting up-front.

**Change:** wrap the heavy bodies in a `LazyOnVisible` helper that:
1. Renders a skeleton placeholder (same height as `estimateSize` predicts).
2. Sets up an `IntersectionObserver` with `rootMargin: "200px"`.
3. On first intersection, swaps the skeleton out for the real component.

This decouples reveal-time cost from total-rendered-count: rows above and below the fold pay zero hydration cost until the user scrolls to them.

**Risk:** the skeleton heights need to match well, otherwise scroll position jumps when bodies hydrate. We already track this via `agentPerfStore.recordEstimatorMeasurement`; can re-use that data to tune skeleton heights per node-kind.

### Phase 3 — Tune `measureElement` cadence (≈40 LOC)

TanStack Virtual measures every rendered row's actual height on first paint, then again on resize. Per the [TanStack docs](https://tanstack.com/virtual/latest/docs/api/virtualizer), the measurement cache is consumed once after mount.

**Change:**
1. On tab reveal, **trust the estimator** for the first frame — skip `measureElement` until after the tab-reveal gate lifts.
2. Then trigger a single measurement pass via `virtualizer.measure()` instead of letting it cascade through the ref callback.

This pulls the layout-thrash off the critical path and lets the user see the document immediately (with estimated heights). Measurements settle in the background a frame later.

**Risk:** if estimator misses are large, the user sees a brief layout shift after reveal. Acceptable trade-off given the perceived freeze is what we're optimizing for.

### Phase 4 (deferred) — Snapshot pre-warm

For tabs visited the first time in a session, `useHistoryPagination` fires an async RPC for `output.state.json`. The pane shows an empty state until the RPC returns.

**Change:** at pane *register* time (before mount), kick off a parallel RPC for the snapshot. By the time the component mounts, the result is either ready or close to ready, so the first paint has content.

**Why deferred:** this affects only first-visit cost; second-visit cost (the dominant complaint) is unaffected. Worth doing eventually but not the first wave.

---

## Sequencing

| Phase | Effort | Expected impact | Risk |
|---|---|---|---|
| 0 — Measure | 0 LOC | Establishes baseline | None |
| 1 — Markdown memo | ~30 LOC + 1 test | High (if hypothesis holds) | Very low — additive memo |
| 2 — Defer expensive renderers | ~80 LOC + tests | High | Medium — skeleton-height accuracy matters |
| 3 — Tune `measureElement` | ~40 LOC + tests | Medium | Medium — layout-shift trade-off |
| 4 — Snapshot pre-warm | ~50 LOC + tests | First-visit only | Low |

Don't skip Phase 0. Each subsequent PR should include a before/after measurement in the PR description, showing the long-task count + duration improved for the case the PR claims to fix. If the measurement doesn't move, the PR doesn't ship.

---

## Acceptance criteria for "tab switch feels snappy"

Numbers are placeholders to be refined after Phase 0 establishes a baseline.

- [ ] **Hot tab-switch (2nd+ visit in same session)** — sum of long-tasks within the tab-reveal gate window is **< 100ms**, dominated by a single `long-task` rather than dozens of small ones.
- [ ] **Cold tab-switch (first visit, snapshot already on disk)** — total time from click-to-paint is **< 500ms**, dominated by the snapshot RPC.
- [ ] **Streaming responsiveness on the inactive tab** doesn't degrade — `useAgentStream`'s RAF flush continues to fire while the tab is hidden; user sees up-to-date document on switch back.
- [ ] **Memory** doesn't spike per additional tab — keeping more bodies alive is fine; observable memory growth per tab should be sublinear with total document size (because virtualization is in play).

---

## Investigation log

### Phase 0 baseline (measured)

Established the 500-600ms long-task pattern per tab switch — see §"Tab-switch durations" earlier. **Critical finding:** the `[perf] tab-switch` mark covers only the reveal-gate window; a separate 500-600ms long-task fires AFTER the gate lifts and is the dominant user-visible freeze. The reveal gate's `tab-switch` number is misleadingly small.

### Phase 1 — markdown memoization (skipped after investigation)

The original hypothesis assumed `<Markdown>` had no memoization. Inspection of `frontend/app/element/markdown.tsx:466` revealed an existing `createMemo` on the unified pipeline keyed via `transformedText() → resolvedText() → props.text`. **The hypothesis was already implemented.** Skipped. No PR shipped.

### Phase 1 (revised) — instrument hot spots (measured negative results)

Added `performance.now()` taps around partition memo, virtualizer `measureElement`, markdown render-memo, and OverlayScrollbars init. Repro showed:

| Suspect | Result | Verdict |
|---|---|---|
| `markdown-parse` | 5 invocations, all cold-boot; 0 during switches | ❌ Memo holds |
| `os-init` | 15 invocations cold-boot only | ❌ Not switch cost |
| `partition` | 0 invocations >5ms | ❌ Cached |
| `measureElement` | 50 calls totaling 0.3ms | ❌ Trivial |
| `DocumentRow` mounts | 3-15 per switch (not hundreds) | ❌ Not For-reconcile cascade |

All five JS-side hypotheses fell. The 500ms long-task is **browser-side layout + paint** on `display:none → block`, opaque to JS-level perf hooks.

### Phase 1 B+C — content-visibility: auto + contain-intrinsic-size (measured NEGATIVE)

Applied `content-visibility: auto` + `contain-intrinsic-size: auto 80px` on `.agent-document-row`. Measured **worse**: 700-1090ms long-tasks (vs 500-600ms baseline). Reverted.

Failure mode: with most streaming-buffer rows near the visible viewport, the browser pays overhead detecting "near-viewport" status without much skipping. AND the 80px placeholder was much smaller than typical row heights (markdown + tool blocks routinely 200-500px), so every reveal triggered layout-shift cascades that re-laid-out the document multiple times.

**Lessons for future retries:**
- Don't blanket-apply `content-visibility: auto` to a list where most items are near-viewport. The skip-detection cost dominates.
- If retrying, base `contain-intrinsic-size` on observed per-kind heights (use `agentPerfStore.recordEstimatorMeasurement` data), not a small constant.

### Where next

JS-side suspects exhausted. Browser-side layout cost remains. Next candidates:
- **Finer profiling via Chrome DevTools Performance tab** — instrument inside Solid's reactive root + the virtualizer's internal effects to find any reactive memo that fires on visibility change and forces a sync layout (forced reflow).
- **Conditional render-on-visible**: only render the active tab's pane content; for inactive tabs, leave a placeholder. Trade-off: cold-mount cost on EVERY switch (probably worse, but worth measuring).
- **Investigate whether the 500ms long-task is actually Chromium's compositor/paint work** — instrument with `requestAnimationFrame` + `performance.now()` around the first paint after tab activation.

---

## Out of scope

- **Splitting individual markdown nodes** into paragraph-level chunks for finer-grained virtualization. Big refactor; pursue only if Phase 1 memo proves insufficient.
- **Web-worker offload** of the unified pipeline. The pipeline isn't slow enough per node to justify the worker round-trip + serialization cost. May reconsider for streaming-token responsiveness if becomes an issue.
- **`solid-keep-alive` integration.** AgentMux doesn't unmount panes via `<Show>` in the relevant code paths, so the keep-alive pattern doesn't apply. Filed away as future option if split-pane navigation changes that.

---

## Open questions

- **Skeleton heights.** For Phase 2, what's the right placeholder for a tool block vs a markdown node? Suggest: same height as the estimator predicts. Confirmable via existing `recordEstimatorMeasurement` data.
- **Markdown cache invalidation.** For Phase 1, when does a finalized markdown node's content actually change? Streaming buffers replace the node object on each token, so the memo key needs to be `props.text` (the string), not `props.node` (the object). Confirmed.
- **First-frame paint policy.** For Phase 3, do we want estimator-only first frame *always*, or only on tab-reveal? Suggest: tab-reveal only, since first scroll within a stable tab already benefits from accurate measurements.

---

## References

### Internal
- `frontend/app/workspace/workspace.tsx:40-48` — display:none + visibility reveal gate
- `frontend/app/tab/tab-reveal.ts` — frame-budget detector that lifts the gate
- `frontend/app/element/markdown.tsx` — unified-pipeline markdown renderer
- `frontend/app/view/agent/virtualization/AgentDocumentVirtualList.tsx` — virtualizer + partition logic
- `frontend/app/view/agent/components/MarkdownBlock.tsx` — per-node markdown wrapper
- `frontend/app/view/agent/components/ToolBlock.tsx` — tool block with eagerly-mounted bodies
- `frontend/perf/index.ts` + `frontend/perf/observers.ts` — existing perf instrumentation

### External — best practices
- [VS Code sandbox migration — keep renderers alive](https://code.visualstudio.com/blogs/2022/11/28/vscode-sandbox)
- [TanStack Virtual docs](https://tanstack.com/virtual/latest/docs/api/virtualizer)
- [LogRocket — speed up long lists with TanStack Virtual](https://blog.logrocket.com/speed-up-long-lists-tanstack-virtual/)
- [StudyRaid — optimizing large Markdown documents](https://app.studyraid.com/en/read/11460/359228/optimizing-large-markdown-documents)
- [Shiki best-performance guide](https://shiki.style/guide/best-performance)
- [Caching Shiki for faster build times](https://dev.to/iamhectorsosa/caching-shiki-for-faster-build-times-4llb)
- [`solid-keep-alive` (Solid's offscreen-component pattern)](https://github.com/JulianSoto/solid-keep-alive)
