// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";
import { __resetDispatchLog, dispatchRecordsAtom } from "./command-source";
import {
    __resetAllSlots,
    dispatch,
    dispatchIfRegistered,
    registerPane,
    snapshot,
    unregisterPane,
    type LayoutView,
} from "./agent-pane-layout-store";
import { DEFAULT_ROW_PX } from "./agent-pane-layout/types";

const BID = "block-123456789";

function mkPane() {
    const layout = vi.fn<(v: LayoutView) => void>();
    const zoom = vi.fn<(z: number) => void>();
    registerPane(BID, { layout, zoom });
    return { layout, zoom };
}

afterEach(() => {
    __resetAllSlots();
    // Reset the SHARED audit ring too, so the audit test isn't order-dependent
    // on whatever earlier tests left in it (reagent P2 on #1236).
    __resetDispatchLog();
});

describe("agent-pane-layout store", () => {
    it("throws on dispatch to an unregistered pane", () => {
        expect(() =>
            dispatch("nope", { type: "ZoomChanged", zoom: 0.5 }),
        ).toThrow(/unregistered pane/);
    });

    it("dispatchIfRegistered is a no-op for an unregistered pane", () => {
        expect(dispatchIfRegistered("nope", { type: "ZoomChanged", zoom: 0.5 })).toEqual([]);
    });

    it("projects the layout view when a layout input changes", () => {
        const { layout } = mkPane();
        dispatch(BID, { type: "NodesChanged", orderedIds: ["a", "b"] });
        dispatch(BID, { type: "RowMeasured", nodeId: "a", state: "collapsed", cssPx: 30 });
        const last = layout.mock.calls.at(-1)![0];
        expect(last.rows.map((r) => r.nodeId)).toEqual(["a", "b"]);
        expect(last.rows[0].height).toBe(30);
        expect(last.totalSize).toBe(30 + DEFAULT_ROW_PX); // measured + default
    });

    it("INV-2: ZoomChanged re-emits zoom but NOT the layout view", () => {
        const { layout, zoom } = mkPane();
        dispatch(BID, { type: "NodesChanged", orderedIds: ["a"] });
        const layoutCallsBefore = layout.mock.calls.length;

        dispatch(BID, { type: "ZoomChanged", zoom: 0.5 });

        expect(zoom).toHaveBeenLastCalledWith(0.5);
        // zero new layout projections — the zoom change costs no relayout
        expect(layout.mock.calls.length).toBe(layoutCallsBefore);
    });

    it("does not re-project on a no-op dispatch", () => {
        const { layout } = mkPane();
        dispatch(BID, { type: "Scrolled", scrollTop: 10, viewportPx: 100 });
        const before = layout.mock.calls.length;
        dispatch(BID, { type: "Scrolled", scrollTop: 10, viewportPx: 100 }); // identical
        expect(layout.mock.calls.length).toBe(before);
    });

    it("does not re-project an off-flow measurement (codex P2)", () => {
        const { layout } = mkPane();
        dispatch(BID, { type: "NodesChanged", orderedIds: ["a"] }); // a is collapsed by default
        dispatch(BID, { type: "RowMeasured", nodeId: "a", state: "collapsed", cssPx: 20 });
        const before = layout.mock.calls.length;
        // Measuring the EXPANDED slot of a collapsed row changes `heights` but
        // not any position → must NOT project a layout change.
        dispatch(BID, { type: "RowMeasured", nodeId: "a", state: "expanded", cssPx: 200 });
        expect(layout.mock.calls.length).toBe(before);
        // …and when the row is later expanded, the cached expanded height applies.
        dispatch(BID, { type: "UserExpanded", nodeId: "a" });
        expect(layout.mock.calls.at(-1)![0].rows[0].height).toBe(200);
    });

    it("records every dispatch in the audit ring with the slice tag + source", () => {
        // afterEach resets the ring, so this starts clean without a manual reset.
        mkPane();
        dispatch(BID, { type: "NodesChanged", orderedIds: ["a"] }, "user");
        const mine = dispatchRecordsAtom().filter((r) => r.slice === "agent-pane-layout");
        expect(mine.length).toBe(1);
        expect(mine[0]).toMatchObject({ key: BID, source: "user" });
    });

    it("unregister + snapshot lifecycle", () => {
        mkPane();
        dispatch(BID, { type: "NodesChanged", orderedIds: ["a"] });
        expect(snapshot(BID)?.orderedIds).toEqual(["a"]);
        unregisterPane(BID);
        expect(snapshot(BID)).toBeNull();
    });

    describe("scroll-only updates reuse the cached prefix sum (task #39)", () => {
        it("a scroll-only dispatch does NOT recompute positions — same `rows` array reference", () => {
            const { layout } = mkPane();
            dispatch(BID, { type: "NodesChanged", orderedIds: ["a", "b", "c"] });
            dispatch(BID, { type: "RowMeasured", nodeId: "a", state: "collapsed", cssPx: 40 });
            const afterData = layout.mock.calls.at(-1)![0];

            dispatch(BID, { type: "Scrolled", scrollTop: 10, viewportPx: 200 });
            const afterScroll = layout.mock.calls.at(-1)![0];

            // `computeLayoutView`/`positions()` allocate a fresh `rows` array
            // on every REAL recompute (documented on `positions()`), so
            // referential equality here proves the scroll-only dispatch
            // reused the cached prefix sum instead of rebuilding it — the
            // O(n)-per-scroll-event bug this fixes.
            expect(afterScroll.rows).toBe(afterData.rows);
            expect(afterScroll.totalSize).toBe(afterData.totalSize);
            // The window itself must still be recomputed (that's the whole
            // point of a scroll) — just via the cheap O(log n) path.
            expect(afterScroll.window).not.toBe(afterData.window);
        });

        it("a data-changing dispatch after a scroll-only one still rebuilds `rows`", () => {
            const { layout } = mkPane();
            dispatch(BID, { type: "NodesChanged", orderedIds: ["a", "b", "c"] });
            dispatch(BID, { type: "Scrolled", scrollTop: 10, viewportPx: 200 });
            const afterScroll = layout.mock.calls.at(-1)![0];

            dispatch(BID, { type: "RowMeasured", nodeId: "b", state: "collapsed", cssPx: 55 });
            const afterMeasure = layout.mock.calls.at(-1)![0];

            expect(afterMeasure.rows).not.toBe(afterScroll.rows);
            expect(afterMeasure.rows[1].height).toBe(55);
        });

        it("the reused-positions window is still correct (not stale) after scrolling", () => {
            const { layout } = mkPane();
            const ids = Array.from({ length: 20 }, (_, i) => `n${i}`);
            dispatch(BID, { type: "NodesChanged", orderedIds: ids });
            for (const id of ids) {
                dispatch(BID, { type: "RowMeasured", nodeId: id, state: "collapsed", cssPx: 20 });
            }
            // Rows are 20px each (0..400 total). viewportPx=40 shows 2 rows;
            // scrollTop=200 puts rows n10/n11 in view, padded by the default
            // overscan (5) on each side.
            dispatch(BID, { type: "Scrolled", scrollTop: 200, viewportPx: 40 });
            const view = layout.mock.calls.at(-1)![0];
            expect(view.window).toEqual({ startIndex: 5, endIndex: 16 });

            // Scroll again (still no data change) — window must track the
            // NEW scrollTop, not the stale cached one.
            dispatch(BID, { type: "Scrolled", scrollTop: 0, viewportPx: 40 });
            const view2 = layout.mock.calls.at(-1)![0];
            expect(view2.window).toEqual({ startIndex: 0, endIndex: 6 });
            // Positions array itself is still the same cached reference.
            expect(view2.rows).toBe(view.rows);
        });
    });
});
