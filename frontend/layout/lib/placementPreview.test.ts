// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";
import { beginDrag, endDrag } from "@/app/drag/drag-session";
import {
    clearPlacement,
    LEAVE_GRACE_MS,
    leavePlacement,
    previewPlacement,
    resetAllDropIntent,
    resetDropIntent,
} from "./placementPreview";
import { dragState } from "./tilelayout-drag-state";
import { DropDirection, LayoutTreeActionType } from "./types";

const reducer = vi.fn();
const model = { treeState: { rootNode: undefined }, treeReducer: reducer } as never;
const rect = { top: 0, left: 0, width: 400, height: 400 };

const input = (targetId: string, x: number, y: number) => ({ targetId, rect, offset: { x, y }, draggedId: "dragged" });

afterEach(() => {
    resetDropIntent(model);
    reducer.mockClear();
    vi.useRealTimers();
    dragState.nodeId = null;
});

describe("previewPlacement", () => {
    it("shows the ghost for the direction under the pointer on the first sample", () => {
        const { direction, targetId } = previewPlacement(model, input("A", 40, 200));
        expect(direction).toBe(DropDirection.OuterLeft);
        expect(targetId).toBe("A");
        expect(reducer).toHaveBeenCalledWith(
            expect.objectContaining({ type: LayoutTreeActionType.ComputeMove, nodeId: "A", direction })
        );
    });

    it("keeps the ghost steady while the pointer rests on a boundary", () => {
        const first = previewPlacement(model, input("A", 150, 200)).direction;
        const seen = new Set<unknown>();
        for (let i = 0; i < 20; i++) {
            seen.add(previewPlacement(model, input("A", 150 + (i % 2 ? 2 : -2), 200)).direction);
        }
        expect(seen).toEqual(new Set([first]));
    });

    it("shows a waiting candidate after the dwell when the pointer stops sending samples", () => {
        vi.useFakeTimers();
        dragState.nodeId = "dragged";
        previewPlacement(model, input("A", 40, 200));
        vi.setSystemTime(Date.now() + 50);
        reducer.mockClear();
        previewPlacement(model, input("B", 3000, 3000));
        vi.advanceTimersByTime(500);
        expect(reducer.mock.calls.length).toBeGreaterThanOrEqual(1);
    });

    it("shows nothing after the drag ended", () => {
        vi.useFakeTimers();
        dragState.nodeId = null;
        previewPlacement(model, input("A", 40, 200));
        reducer.mockClear();
        previewPlacement(model, input("B", 3000, 3000));
        reducer.mockClear();
        vi.advanceTimersByTime(500);
        expect(reducer).not.toHaveBeenCalled();
    });

    it("shows nothing when the ghost was cleared while a candidate waited", () => {
        vi.useFakeTimers();
        dragState.nodeId = "dragged";
        previewPlacement(model, input("A", 40, 200));
        vi.setSystemTime(Date.now() + 50);
        previewPlacement(model, input("B", 3000, 3000));
        clearPlacement(model);
        reducer.mockClear();
        vi.advanceTimersByTime(500);
        expect(reducer).not.toHaveBeenCalled();
    });

    it("starts afresh after the pointer was over the dragged pane", () => {
        previewPlacement(model, input("A", 40, 200));
        clearPlacement(model, true);
        const { targetId } = previewPlacement(model, input("B", 40, 200));
        expect(targetId).toBe("B");
    });

    it("starts afresh when the tree changes under the drag", () => {
        // Two panes, hit-tested against the model's own rects; then srv swaps their places mid-drag.
        let shape = [
            { id: "A", rect: { top: 0, left: 0, width: 400, height: 400 } },
            { id: "B", rect: { top: 0, left: 400, width: 400, height: 400 } },
        ];
        const changing = {
            treeState: { rootNode: undefined },
            treeReducer: vi.fn(),
            leafs: () => shape.map((s) => ({ id: s.id })),
            getNodeRectById: (id: string) => shape.find((s) => s.id === id)?.rect,
        } as never;
        previewPlacement(changing, input("A", 40, 200));
        // Still the same layout: a fast jump over B is held back by the tracker, as before.
        expect(previewPlacement(changing, input("B", 3000, 3000)).targetId).toBe("A");
        shape = [shape[1], shape[0]].map((s, i) => ({ ...s, rect: { ...s.rect, left: i * 400 } }));
        // A new layout: the first sample on it arms at once, whatever was armed on the old one.
        expect(previewPlacement(changing, input("B", 3000, 3000)).targetId).toBe("B");
        resetDropIntent(changing);
    });

    const cleared = () => reducer.mock.calls.filter(([a]) => a.type === LayoutTreeActionType.ClearPendingAction).length;

    it("keeps the drop shown when the pointer only crosses to the next pane's zones", () => {
        vi.useFakeTimers();
        beginDrag("tile");
        previewPlacement(model, input("A", 40, 200));
        // The leave of A's overlay, then B's first sample a frame later: nothing was cleared in between.
        leavePlacement(model);
        expect(cleared()).toBe(0);
        vi.advanceTimersByTime(16);
        previewPlacement(model, input("B", 44, 200));
        vi.advanceTimersByTime(LEAVE_GRACE_MS * 2);
        expect(cleared()).toBe(0);
        endDrag("drop");
    });

    it("clears the drop once the pointer has really left: no sample within the grace", () => {
        vi.useFakeTimers();
        beginDrag("tile");
        previewPlacement(model, input("A", 40, 200));
        leavePlacement(model);
        vi.advanceTimersByTime(LEAVE_GRACE_MS - 1);
        expect(cleared()).toBe(0);
        vi.advanceTimersByTime(1);
        expect(cleared()).toBe(1);
        endDrag("cancel");
    });

    it("clears at once what a grace still shows when the drag is over, and waits for nothing with no drag on", () => {
        vi.useFakeTimers();
        beginDrag("tile");
        previewPlacement(model, input("A", 40, 200));
        leavePlacement(model);
        // A cancelled drag ends with a leave: its end clears the drop.
        endDrag("cancel");
        expect(cleared()).toBe(1);
        vi.advanceTimersByTime(LEAVE_GRACE_MS * 2);
        expect(cleared()).toBe(1);
        leavePlacement(model);
        expect(cleared()).toBe(2);
    });

    it("resets every tab's steadying at the end of a drag", () => {
        const other = { treeState: { rootNode: undefined }, treeReducer: vi.fn() } as never;
        previewPlacement(other, input("A", 40, 200));
        resetAllDropIntent();
        expect(previewPlacement(other, input("B", 40, 200)).targetId).toBe("B");
        resetDropIntent(other);
    });
});
