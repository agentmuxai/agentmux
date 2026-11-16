// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The one place a pointer becomes a drop direction and a shown drop: the tile overlay and a tab pill over a
 * pane's body both call it, so for the same point they get the same direction. A drop is under the pointer
 * only on its spot of the hovered pane's guide (dropGuides.ts).
 */

import { describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { guideIn } from "@/layout/lib/dropGuides";
import { resetDropOptions } from "@/layout/lib/dropOptions";
import { LayoutModel } from "@/layout/lib/layoutModel";
import { pillStandIn } from "@/layout/lib/paneBodyDrop";
import { placementDirection, previewPlacement, resetDropIntent, sideOfDirection, type PlacementInput } from "@/layout/lib/placementPreview";
import { DropDirection, FlexDirection, LayoutTreeActionType, type LayoutNode } from "@/layout/lib/types";
import type { SignalAtom } from "@/util/util";

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

/** Three panes in a row, in a 1200 by 800 layout. */
function threeInARow() {
    resetDropOptions();
    const [tab] = createSignal<Tab>({ otype: "tab", oid: "tab-1", version: 1, meta: {}, name: "t", layoutstate: "layout-1", blockids: [] });
    const m = new LayoutModel(tab);
    const box = { top: 0, left: 0, x: 0, y: 0, width: 1200, height: 800 };
    m.getBoundingRect = () => box;
    m.displayContainerRef.current = { getBoundingClientRect: () => box } as any;
    m.treeState.rootNode = { id: "r", size: 10, flexDirection: FlexDirection.Row, children: [leaf("a"), leaf("b"), leaf("c")] } as LayoutNode;
    m.updateTree();
    resetDropIntent(m);
    return m;
}

/** A drag of `subject` over the pane `targetId`, with the pointer at `offset`. */
const over = (m: LayoutModel, targetId: string, offset: { x: number; y: number }, subject: Partial<PlacementInput>): PlacementInput =>
    ({ targetId, rect: m.getNodeRectById(targetId), offset, draggedId: "a", ...subject }) as PlacementInput;

/** The middle of the spot for `direction` in the guide that is shown. */
function spot(m: LayoutModel, direction: DropDirection): { x: number; y: number } {
    const found = guideIn(m)?.spots.find((s) => s.direction === direction);
    if (!found) throw new Error(`no spot for ${DropDirection[direction]}`);
    return { x: found.x, y: found.y };
}

const corner = (m: LayoutModel, id: string) => {
    const r = m.getNodeRectById(id);
    return { x: r.left + 6, y: r.top + 6 };
};

describe("placementDirection", () => {
    it("is the direction of the spot under the pointer, and none anywhere else in the pane", () => {
        const m = threeInARow();
        // A pane of the tree over the far pane of its row: every side, the swap, and nothing else.
        expect(placementDirection(m, over(m, "c", corner(m, "c"), {}))).toBeUndefined();
        const directions = guideIn(m)!.spots.map((s) => s.direction);
        expect(directions).toEqual([DropDirection.Top, DropDirection.Right, DropDirection.Bottom, DropDirection.Left, DropDirection.Center]);
        for (const direction of directions) {
            expect(placementDirection(m, over(m, "c", spot(m, direction), {}))).toBe(direction);
            expect(guideIn(m)!.active).toBe(direction);
        }
        expect(placementDirection(m, over(m, "c", corner(m, "c"), {}))).toBeUndefined();
        expect(guideIn(m)!.active).toBeUndefined();
    });

    it("offers no spot for a drop that leaves the layout as it is", () => {
        const m = threeInARow();
        // `a` already stands to the left of `b`.
        placementDirection(m, over(m, "b", corner(m, "b"), {}));
        expect(guideIn(m)!.spots.map((s) => s.direction)).toEqual([DropDirection.Top, DropDirection.Right, DropDirection.Bottom]);
    });

    it("gives a tab pill the four sides and the join, which a release off the spots makes too", () => {
        const m = threeInARow();
        const stand = pillStandIn("x");
        const pill = { draggedId: stand.id, foreignNode: stand, pill: true };
        expect(placementDirection(m, over(m, "b", corner(m, "b"), pill))).toBeUndefined();
        const guide = guideIn(m)!;
        expect(guide.spots.map((s) => s.direction)).toEqual([
            DropDirection.Top,
            DropDirection.Right,
            DropDirection.Bottom,
            DropDirection.Left,
            DropDirection.Center,
        ]);
        // Off the spots the tab joins the pane's tabs: the join's spot is the lit one.
        expect(guide.active).toBe(DropDirection.Center);
        expect(placementDirection(m, over(m, "b", spot(m, DropDirection.Left), pill))).toBe(DropDirection.Left);
        expect(guideIn(m)!.active).toBe(DropDirection.Left);
    });

    it("keeps the same spots for as long as the pointer stays over one pane", () => {
        const m = threeInARow();
        placementDirection(m, over(m, "c", corner(m, "c"), {}));
        const spots = guideIn(m)!.spots;
        placementDirection(m, over(m, "c", spot(m, DropDirection.Top), {}));
        expect(guideIn(m)!.spots).toBe(spots);
        placementDirection(m, over(m, "b", corner(m, "b"), {}));
        expect(guideIn(m)!.spots).not.toBe(spots);
        expect(guideIn(m)!.targetId).toBe("b");
    });
});

describe("previewPlacement", () => {
    it("shows a node that is not in the tree as the split of the hovered pane, and says the move it stands for", () => {
        const m = threeInARow();
        const foreign = { ...leaf("other-tab-pane") };
        const subject = { draggedId: foreign.id, foreignNode: foreign };
        placementDirection(m, over(m, "b", corner(m, "b"), subject));
        const treeReducer = vi.spyOn(m, "treeReducer");
        const { direction, compute, targetId } = previewPlacement(m, over(m, "b", spot(m, DropDirection.Bottom), subject));
        expect(direction).toBe(DropDirection.Bottom);
        expect(targetId).toBe("b");
        // A pane from another window tab lands by srv's split of the hovered pane: that is what is shown.
        expect(treeReducer).toHaveBeenCalledWith({
            type: LayoutTreeActionType.SetPendingAction,
            action: expect.objectContaining({
                type: LayoutTreeActionType.SplitVertical,
                targetNodeId: "b",
                newNode: foreign,
                position: "after",
                sizeFraction: 0.5,
            }),
        });
        expect(compute).toMatchObject({
            type: LayoutTreeActionType.ComputeMove,
            nodeId: "b",
            nodeToMoveId: foreign.id,
            direction: DropDirection.Bottom,
            nodeToMove: foreign,
        });
    });

    it("carries no node for one that is in the tree", () => {
        const m = threeInARow();
        placementDirection(m, over(m, "c", corner(m, "c"), {}));
        const { compute, direction } = previewPlacement(m, over(m, "c", spot(m, DropDirection.Center), {}));
        expect(direction).toBe(DropDirection.Center);
        expect((compute as { nodeToMove?: unknown }).nodeToMove).toBeUndefined();
    });

    it("shows a tab that leaves others behind as the split of the hovered pane", () => {
        const m = threeInARow();
        const stand = pillStandIn("x");
        const pill = { draggedId: stand.id, foreignNode: stand, pill: true };
        placementDirection(m, over(m, "b", corner(m, "b"), pill));
        const treeReducer = vi.spyOn(m, "treeReducer");
        previewPlacement(m, over(m, "b", spot(m, DropDirection.Right), pill));
        expect(treeReducer).toHaveBeenCalledWith({
            type: LayoutTreeActionType.SetPendingAction,
            action: expect.objectContaining({
                type: LayoutTreeActionType.SplitHorizontal,
                targetNodeId: "b",
                newNode: stand,
                position: "after",
                sizeFraction: 0.5,
            }),
        });
    });

    it("lights the spot of the drop that is shown", () => {
        const m = threeInARow();
        placementDirection(m, over(m, "c", corner(m, "c"), {}));
        previewPlacement(m, over(m, "c", spot(m, DropDirection.Top), {}));
        expect(guideIn(m)!.active).toBe(DropDirection.Top);
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
