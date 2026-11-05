// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";
import { previewPlacement, resetDropIntent } from "./placementPreview";
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
});
