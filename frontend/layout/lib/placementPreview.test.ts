// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a drag keeps between samples (placementPreview.ts): the steadied candidate, its dwell timer, and the
 * grace a leave waits. On a real layout model, with the pointer on the spots of the hovered pane's guide
 * (dropGuides.ts), which is where a drop is.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { beginDrag, endDrag } from "@/app/drag/drag-session";
import type { SignalAtom } from "@/util/util";
import { guideSpotsFor } from "./dropGuides";
import { dropOptionsOf, resetDropOptions } from "./dropOptions";
import { LayoutModel } from "./layoutModel";
import {
    clearPlacement,
    LEAVE_GRACE_MS,
    leavePlacement,
    previewPlacement,
    resetAllDropIntent,
    resetDropIntent,
} from "./placementPreview";
import { dragState } from "./tilelayout-drag-state";
import { DropDirection, FlexDirection, LayoutTreeActionType, type LayoutNode } from "./types";

const rows = new Map<string, SignalAtom<LayoutState>>();
vi.mock("@/app/store/rpc-util", () => ({
    TabRpcClient: { rpcCall: async () => ({ applied: true, version: Number.MAX_SAFE_INTEGER }) },
}));
vi.mock("@/app/store/global", () => ({
    MOS: {
        makeORef: (_otype: string, oid: string) => oid,
        getMuxObjectAtom: (oid: string) => {
            if (!rows.has(oid)) {
                const [get, set] = createSignal<LayoutState>({ otype: "layout", oid, version: 1, meta: {} } as LayoutState);
                const atom = () => get();
                (atom as any)._set = set;
                rows.set(oid, atom as unknown as SignalAtom<LayoutState>);
            }
            return rows.get(oid);
        },
        getObjectValue: () => undefined,
        setObjectValue: () => {},
    },
    getSettingsKeyAtom: () => createSignal(0.75)[0],
    globalStore: { get: () => undefined, set: () => {} },
}));
vi.mock("@/app/store/focusManager", () => ({
    focusManager: { requestNodeFocus: () => {}, refocusNode: () => {}, claimFocusOnMount: () => {}, ensureSelectionFocused: () => {} },
}));

// A pane carries the axis it would split along, the other one than its row's (`addChildAt`, layoutNode.ts).
const leaf = (id: string): LayoutNode => ({ id, size: 10, flexDirection: FlexDirection.Column, data: { blockId: id } }) as LayoutNode;

const made: LayoutModel[] = [];

/** Panes A, B and the dragged one, in a row, in a 1200 by 400 layout. */
function layout() {
    resetDropOptions();
    rows.clear();
    const [tab] = createSignal<Tab>({ otype: "tab", oid: "tab-1", version: 1, meta: {}, name: "t", layoutstate: "layout-1", blockids: [] });
    const m = new LayoutModel(tab);
    const box = { top: 0, left: 0, x: 0, y: 0, width: 1200, height: 400 };
    m.getBoundingRect = () => box;
    m.displayContainerRef.current = { getBoundingClientRect: () => box } as any;
    m.treeState.rootNode = { id: "r", size: 10, flexDirection: FlexDirection.Row, children: [leaf("A"), leaf("B"), leaf("dragged")] } as LayoutNode;
    m.updateTree();
    // srv stores the same tree: a quiet model shows what srv stores (layoutIntents.ts).
    const stored = { otype: "layout", oid: "layout-1", version: 1, meta: {}, rootnode: structuredClone(m.treeState.rootNode) };
    (rows.get("layout-1") as unknown as { _set: (row: unknown) => void })?._set(stored);
    made.push(m);
    return { m, reducer: vi.spyOn(m, "treeReducer") };
}

/** One sample of the drag over `targetId`, with the pointer `dx` px from the middle of its `direction` spot. */
function on(m: LayoutModel, targetId: string, direction: DropDirection, dx = 0) {
    const rect = m.getNodeRectById(targetId);
    const spot = guideSpotsFor(m, rect, targetId, dropOptionsOf(m, { targetId, draggedId: "dragged" })).find((s) => s.direction === direction);
    if (!spot) throw new Error(`no ${DropDirection[direction]} spot over ${targetId}`);
    return { targetId, rect, offset: { x: spot.x + dx, y: spot.y }, draggedId: "dragged", spot };
}

afterEach(() => {
    for (const m of made.splice(0)) resetDropIntent(m);
    vi.useRealTimers();
    dragState.nodeId = null;
});

describe("previewPlacement", () => {
    it("shows the ghost for the spot under the pointer on the first sample", () => {
        const { m, reducer } = layout();
        const { direction, targetId } = previewPlacement(m, on(m, "A", DropDirection.Left));
        expect(direction).toBe(DropDirection.Left);
        expect(targetId).toBe("A");
        expect(reducer).toHaveBeenCalledWith(
            expect.objectContaining({ type: LayoutTreeActionType.ComputeMove, nodeId: "A", direction })
        );
    });

    it("keeps the ghost steady while the pointer rests on a spot's edge", () => {
        const { m } = layout();
        // The far edge of the left spot's reach: two px either way is on the spot, then off it.
        const edge = -(on(m, "A", DropDirection.Left).spot.width / 2 + 5);
        const first = previewPlacement(m, on(m, "A", DropDirection.Left, edge + 2)).direction;
        expect(first).toBe(DropDirection.Left);
        const seen = new Set<unknown>();
        for (let i = 0; i < 20; i++) {
            seen.add(previewPlacement(m, on(m, "A", DropDirection.Left, edge + (i % 2 ? 2 : -2))).direction);
        }
        expect(seen).toEqual(new Set([first]));
    });

    it("shows a waiting candidate after the dwell when the pointer stops sending samples", () => {
        vi.useFakeTimers();
        const { m, reducer } = layout();
        dragState.nodeId = "dragged";
        previewPlacement(m, on(m, "A", DropDirection.Left));
        vi.setSystemTime(Date.now() + 50);
        reducer.mockClear();
        previewPlacement(m, on(m, "B", DropDirection.Top));
        vi.advanceTimersByTime(500);
        expect(reducer.mock.calls.length).toBeGreaterThanOrEqual(1);
    });

    it("shows nothing after the drag ended", () => {
        vi.useFakeTimers();
        const { m, reducer } = layout();
        dragState.nodeId = null;
        previewPlacement(m, on(m, "A", DropDirection.Left));
        reducer.mockClear();
        previewPlacement(m, on(m, "B", DropDirection.Top));
        reducer.mockClear();
        vi.advanceTimersByTime(500);
        expect(reducer).not.toHaveBeenCalled();
    });

    it("shows nothing when the ghost was cleared while a candidate waited", () => {
        vi.useFakeTimers();
        const { m, reducer } = layout();
        dragState.nodeId = "dragged";
        previewPlacement(m, on(m, "A", DropDirection.Left));
        vi.setSystemTime(Date.now() + 50);
        previewPlacement(m, on(m, "B", DropDirection.Top));
        clearPlacement(m);
        reducer.mockClear();
        vi.advanceTimersByTime(500);
        expect(reducer).not.toHaveBeenCalled();
    });

    it("starts afresh after the pointer was over the dragged pane", () => {
        const { m } = layout();
        previewPlacement(m, on(m, "A", DropDirection.Left));
        clearPlacement(m, true);
        const { targetId } = previewPlacement(m, on(m, "B", DropDirection.Top));
        expect(targetId).toBe("B");
    });

    it("starts afresh when the tree changes under the drag", () => {
        const { m } = layout();
        previewPlacement(m, on(m, "A", DropDirection.Left));
        // Still the same layout: a fast jump over B is held back by the tracker.
        expect(previewPlacement(m, on(m, "B", DropDirection.Top)).targetId).toBe("A");
        // srv resizes a pane mid-drag: every rect the drag was hit-tested against is another.
        m.treeState.rootNode.children![0].size = 30;
        m.updateTree();
        // A new layout: the first sample on it arms at once, whatever was armed on the old one.
        expect(previewPlacement(m, on(m, "B", DropDirection.Top)).targetId).toBe("B");
    });

    const cleared = (reducer: { mock: { calls: unknown[][] } }) =>
        reducer.mock.calls.filter(([a]) => (a as { type: LayoutTreeActionType }).type === LayoutTreeActionType.ClearPendingAction).length;

    it("keeps the drop shown when the pointer only crosses to the next pane's zones", () => {
        vi.useFakeTimers();
        const { m, reducer } = layout();
        beginDrag("tile");
        previewPlacement(m, on(m, "A", DropDirection.Right));
        // The leave of A's overlay, then B's first sample a frame later: nothing was cleared in between.
        leavePlacement(m);
        expect(cleared(reducer)).toBe(0);
        vi.advanceTimersByTime(16);
        previewPlacement(m, on(m, "B", DropDirection.Left));
        vi.advanceTimersByTime(LEAVE_GRACE_MS * 2);
        expect(cleared(reducer)).toBe(0);
        endDrag("drop");
    });

    it("clears the drop once the pointer has really left: no sample within the grace", () => {
        vi.useFakeTimers();
        const { m, reducer } = layout();
        beginDrag("tile");
        previewPlacement(m, on(m, "A", DropDirection.Left));
        leavePlacement(m);
        vi.advanceTimersByTime(LEAVE_GRACE_MS - 1);
        expect(cleared(reducer)).toBe(0);
        vi.advanceTimersByTime(1);
        expect(cleared(reducer)).toBe(1);
        endDrag("cancel");
    });

    it("clears at once what a grace still shows when the drag is over, and waits for nothing with no drag on", () => {
        vi.useFakeTimers();
        const { m, reducer } = layout();
        beginDrag("tile");
        previewPlacement(m, on(m, "A", DropDirection.Left));
        leavePlacement(m);
        // A cancelled drag ends with a leave: its end clears the drop.
        endDrag("cancel");
        expect(cleared(reducer)).toBe(1);
        vi.advanceTimersByTime(LEAVE_GRACE_MS * 2);
        expect(cleared(reducer)).toBe(1);
        leavePlacement(m);
        expect(cleared(reducer)).toBe(2);
    });

    it("resets every tab's steadying at the end of a drag", () => {
        const { m: other } = layout();
        previewPlacement(other, on(other, "A", DropDirection.Left));
        resetAllDropIntent();
        expect(previewPlacement(other, on(other, "B", DropDirection.Top)).targetId).toBe("B");
    });
});
