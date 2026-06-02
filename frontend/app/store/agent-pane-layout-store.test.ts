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
});
