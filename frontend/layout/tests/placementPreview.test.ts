// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

/**
 * The one place a pointer becomes a drop direction and a ghost: the tile overlay and a tab pill over a
 * pane's body both call it, so for the same point they get the same direction.
 * docs/specs/SPEC_PANE_TAB_DRAG_BODY_PLACEMENT_AND_EDGE_GLOW_2026_10_29.md §3.2.
 */

import { describe, expect, it, vi } from "vitest";
import type { LayoutModel } from "@/layout/lib/layoutModel";
import { placementDirection, previewPlacement, sideOfDirection } from "@/layout/lib/placementPreview";
import { determineDropDirection } from "@/layout/lib/utils";
import { DropDirection, LayoutTreeActionType, type LayoutNode } from "@/layout/lib/types";

const RECT = { left: 0, top: 0, width: 500, height: 400 };
const leaf = (id: string) => ({ id, data: { blockId: id } }) as LayoutNode;
const model = () => {
    const treeReducer = vi.fn();
    return {
        model: { treeState: { rootNode: leaf("p1") }, treeReducer } as unknown as LayoutModel,
        treeReducer,
    };
};

describe("placementDirection", () => {
    it("for a node that is in the tree, is the pane drag's direction, outer bands included", () => {
        const { model: m } = model();
        const offset = { x: 20, y: 200 };
        expect(placementDirection(m, { targetId: "p1", rect: RECT, offset, draggedId: "p2" })).toBe(
            determineDropDirection(RECT, offset)
        );
        expect(determineDropDirection(RECT, offset)).toBe(DropDirection.OuterLeft);
    });

    it("for a node that is not (a tab pill), clamps an outer band to the inner direction", () => {
        const { model: m } = model();
        expect(
            placementDirection(m, {
                targetId: "p1",
                rect: RECT,
                offset: { x: 20, y: 200 },
                draggedId: "pill:b",
                foreignNode: leaf("pill:b"),
            })
        ).toBe(DropDirection.Left);
    });

    it("is undefined outside the pane", () => {
        const { model: m } = model();
        expect(placementDirection(m, { targetId: "p1", rect: RECT, offset: { x: -5, y: 10 }, draggedId: "p2" })).toBeUndefined();
    });
});

describe("previewPlacement", () => {
    it("dispatches ComputeMove for the direction, carrying a foreign node", () => {
        const { model: m, treeReducer } = model();
        const stand = leaf("pill:b");
        const { direction, compute } = previewPlacement(m, {
            targetId: "p1",
            rect: RECT,
            offset: { x: 250, y: 380 },
            draggedId: "pill:b",
            foreignNode: stand,
        });
        expect(direction).toBe(DropDirection.Bottom);
        expect(treeReducer).toHaveBeenCalledWith(compute);
        expect(compute).toMatchObject({
            type: LayoutTreeActionType.ComputeMove,
            nodeId: "p1",
            nodeToMoveId: "pill:b",
            direction: DropDirection.Bottom,
            nodeToMove: stand,
        });
    });

    it("carries no node for one that is in the tree", () => {
        const { model: m } = model();
        const { compute } = previewPlacement(m, { targetId: "p1", rect: RECT, offset: { x: 250, y: 200 }, draggedId: "p2" });
        expect((compute as { nodeToMove?: unknown }).nodeToMove).toBeUndefined();
    });
});

describe("sideOfDirection", () => {
    it("maps every inner and outer direction to a side, and the centre to none", () => {
        expect(sideOfDirection(DropDirection.Left)).toBe("left");
        expect(sideOfDirection(DropDirection.OuterLeft)).toBe("left");
        expect(sideOfDirection(DropDirection.Right)).toBe("right");
        expect(sideOfDirection(DropDirection.OuterRight)).toBe("right");
        expect(sideOfDirection(DropDirection.Top)).toBe("up");
        expect(sideOfDirection(DropDirection.OuterTop)).toBe("up");
        expect(sideOfDirection(DropDirection.Bottom)).toBe("down");
        expect(sideOfDirection(DropDirection.OuterBottom)).toBe("down");
        expect(sideOfDirection(DropDirection.Center)).toBeUndefined();
        expect(sideOfDirection(undefined)).toBeUndefined();
    });
});
