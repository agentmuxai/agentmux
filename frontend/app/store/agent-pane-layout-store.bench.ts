// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Benchmarks for `agent-pane-layout-store.ts` dispatch — task #40, the
 * empirical follow-up to the task #39 fix.
 *
 * Manual-only: NOT wired into `package.json`/CI. Run on demand with:
 *
 *   npx vitest bench frontend/app/store/agent-pane-layout-store.bench.ts
 *
 * This is the first `.bench.ts` file in this repo — establishing the
 * convention (Vitest's `bench()`/`describe()` from the `vitest` package,
 * one `describe` per scenario, one `bench` per input size, run via
 * `vitest bench` rather than `vitest run`).
 *
 * What this measures: `dispatch()` cost for a SCROLL-ONLY update
 * (`scrollTop` changes, nothing else) against a pane whose row positions
 * are already built and cached, at 100 / 1,000 / 10,000 rows.
 *
 * Before task #39's fix, `layoutInputsChanged` gated the full
 * `computeLayoutView()` — which rebuilds the O(n) prefix-sum
 * `positions()` array — on ANY of `orderedIds/heights/estimates/
 * scrollTop/viewportPx/scrollMarginPx/overscan` changing. A pure scroll
 * event re-walked the ENTIRE historical row-position array every time,
 * so this benchmark's cost would have scaled ~linearly with row count
 * (100 → 1,000 → 10,000 roughly 10x → 100x baseline). We don't maintain
 * that old code path here to compare against directly (git history has
 * it, and `positions()`'s own doc comment still says "O(n)") — this
 * bench exists to prove the CURRENT code doesn't regress back into that
 * shape, and to give a concrete "is a scroll-only dispatch cheap"
 * number for future changes to check themselves against.
 *
 * After the fix: a scroll-only dispatch reuses the cached `rows` array
 * and only re-derives the visible window via `windowRangeOf`'s O(log n)
 * binary search (see `positionInputsChanged` / `windowInputsChanged` in
 * `agent-pane-layout-store.ts`). Expect near-flat timing across row
 * counts — any noticeable growth with row count here is a regression
 * signal (the O(n) rebuild came back).
 */

import { bench, describe } from "vitest";
import { dispatch, registerPane } from "./agent-pane-layout-store";

const ROW_COUNTS = [100, 1_000, 10_000] as const;

/** Register a pane with `rowCount` rows already positioned (one
 *  `NodesChanged` dispatch — the only O(n) work this setup needs, and
 *  it happens ONCE, outside the timed `bench()` loop below). Default
 *  row heights are fine: this benchmark is about the SCROLL path, not
 *  measurement/estimation. */
function setupPane(rowCount: number): string {
    const blockId = `bench-scroll-${rowCount}`;
    registerPane(blockId, {
        layout: () => { /* no-op projection — bench doesn't render */ },
        zoom: () => { /* no-op projection */ },
    });
    const orderedIds = Array.from({ length: rowCount }, (_, i) => `n${i}`);
    dispatch(blockId, { type: "NodesChanged", orderedIds });
    dispatch(blockId, { type: "Scrolled", scrollTop: 0, viewportPx: 600 });
    return blockId;
}

describe("agent-pane-layout-store dispatch: scroll-only update", () => {
    for (const rowCount of ROW_COUNTS) {
        const blockId = setupPane(rowCount);
        // Every call uses a DIFFERENT scrollTop. If we sent the same value
        // twice, the reducer's own `Scrolled` no-op short-circuit
        // (`state.scrollTop === command.scrollTop`) would return the
        // identical state ref and the store would skip its window-changed
        // branch entirely — cheating the benchmark by measuring the no-op
        // path instead of the real "recompute the window" path.
        let scrollTop = 0;
        bench(`${rowCount.toLocaleString()} rows`, () => {
            scrollTop = (scrollTop + 17) % 1_000_000;
            dispatch(blockId, { type: "Scrolled", scrollTop, viewportPx: 600 });
        });
    }
});
