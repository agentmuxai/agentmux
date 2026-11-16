// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

/**
 * What a dragged tab pill does over a pane: its header and its body join the pill to the pane's tabs, and a
 * spot of the pane's guide (dropGuides.ts) places it as a pane of its own on that side.
 * docs/specs/SPEC_PANE_TAB_DRAG_BODY_PLACEMENT_AND_EDGE_GLOW_2026_10_29.md §3.1.
 */

import { describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import { guideIn } from "@/layout/lib/dropGuides";
import { resetDropOptions } from "@/layout/lib/dropOptions";
import { LayoutModel } from "@/layout/lib/layoutModel";
import { clearPillGhost, pillStandIn, pillSubject, pillZone, showPillGhost, type PillZoneInput } from "@/layout/lib/paneBodyDrop";
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

/** Where the layout's container sits in the window. */
const ORIGIN = { x: 10, y: 20 };

// A pane carries the axis it would split along, the other one than its row's (`addChildAt`, layoutNode.ts).
const leaf = (id: string, blockId: string): LayoutNode =>
    ({ id, size: 10, flexDirection: FlexDirection.Column, data: { blockId } }) as LayoutNode;

/** Two panes side by side, `p1` (block `a`) and `p2` (block `b`), in a container at ORIGIN in the window. */
function twoPanes(): LayoutModel {
    resetDropOptions();
    const [tab] = createSignal<Tab>({ otype: "tab", oid: "tab-1", version: 1, meta: {}, name: "t", layoutstate: "layout-1", blockids: [] });
    const m = new LayoutModel(tab);
    m.getBoundingRect = () => ({ top: 0, left: 0, width: 1200, height: 800 });
    m.displayContainerRef.current = {
        getBoundingClientRect: () => ({ ...ORIGIN, left: ORIGIN.x, top: ORIGIN.y, width: 1200, height: 800 }),
    } as any;
    m.treeState.rootNode = { id: "r", size: 10, flexDirection: FlexDirection.Row, children: [leaf("p1", "a"), leaf("p2", "b")] } as LayoutNode;
    m.updateTree();
    return m;
}

/** The header band of `p1`, in the window's coordinates. */
function headerOf(m: LayoutModel) {
    const r = m.getNodeRectById("p1");
    const [left, top] = [ORIGIN.x + r.left, ORIGIN.y + r.top];
    return { left, top, right: left + r.width, bottom: top + 33 };
}

/** A tab that is in no pane of this layout (`x`), dragged over `p1`. */
function input(m: LayoutModel, pointer: { x: number; y: number }, extra: Partial<PillZoneInput> = {}): PillZoneInput {
    return { model: m, nodeId: "p1", blockId: "x", pointer, headerRect: headerOf(m), ownPane: false, hasBody: true, ...extra };
}

/** A point of `p1`'s body off every spot, in the window's coordinates. */
function offSpots(m: LayoutModel) {
    const r = m.getNodeRectById("p1");
    return { x: ORIGIN.x + r.left + 8, y: ORIGIN.y + r.top + r.height - 8 };
}

/** The middle of the spot for `direction` in the guide that is shown, in the window's coordinates. */
function spot(m: LayoutModel, direction: DropDirection) {
    const found = guideIn(m)?.spots.find((s) => s.direction === direction);
    if (!found) throw new Error(`no spot for ${DropDirection[direction]}`);
    return { x: ORIGIN.x + found.x, y: ORIGIN.y + found.y };
}

describe("pillZone", () => {
    it("the header joins the pill to the pane's tabs, wherever in the header", () => {
        const m = twoPanes();
        const h = headerOf(m);
        expect(pillZone(input(m, { x: h.left + 20, y: h.top + 10 }))).toEqual({ kind: "join" });
        expect(pillZone(input(m, { x: h.right - 10, y: h.bottom - 1 }))).toEqual({ kind: "join" });
    });

    it("the body joins too, off the spots and on the middle one", () => {
        const m = twoPanes();
        expect(pillZone(input(m, offSpots(m)))).toEqual({ kind: "join" });
        expect(guideIn(m)!.active).toBe(DropDirection.Center);
        expect(pillZone(input(m, spot(m, DropDirection.Center)))).toEqual({ kind: "join" });
    });

    it("a side's spot places the pill as a pane on that side", () => {
        const m = twoPanes();
        pillZone(input(m, offSpots(m)));
        const place = (direction: DropDirection) => pillZone(input(m, spot(m, direction)));
        expect(place(DropDirection.Left)).toEqual({ kind: "place", direction: DropDirection.Left, side: "left" });
        expect(place(DropDirection.Right)).toEqual({ kind: "place", direction: DropDirection.Right, side: "right" });
        expect(place(DropDirection.Top)).toEqual({ kind: "place", direction: DropDirection.Top, side: "up" });
        expect(place(DropDirection.Bottom)).toEqual({ kind: "place", direction: DropDirection.Bottom, side: "down" });
    });

    it("offers no drop beside the pane's whole row: the release can only split the pane", () => {
        const m = twoPanes();
        pillZone(input(m, offSpots(m)));
        for (const s of guideIn(m)!.spots) expect(s.direction < DropDirection.OuterTop || s.direction === DropDirection.Center).toBe(true);
    });

    it("a pill over its own pane can be placed but never joined to where it already is", () => {
        const m = twoPanes();
        const h = headerOf(m);
        expect(pillZone(input(m, { x: h.left + 20, y: h.top + 10 }, { ownPane: true }))).toEqual({ kind: "none" });
        expect(pillZone(input(m, offSpots(m), { ownPane: true }))).toEqual({ kind: "none" });
        // No join's spot, and none lit.
        expect(guideIn(m)!.spots.map((s) => s.direction)).toEqual([DropDirection.Top, DropDirection.Right, DropDirection.Bottom, DropDirection.Left]);
        expect(guideIn(m)!.active).toBeUndefined();
        expect(pillZone(input(m, spot(m, DropDirection.Left), { ownPane: true }))).toMatchObject({ kind: "place", side: "left" });
    });

    it("a pane's only tab is offered the drops its pane's move makes, so none where the pane already stands", () => {
        const m = twoPanes();
        // `b` is the only tab of `p2`, which stands to the right of `p1`.
        expect(pillSubject(m, "b")).toMatchObject({ sourceNodeId: "p2", lone: true, draggedId: "p2" });
        pillZone(input(m, offSpots(m), { blockId: "b" }));
        expect(guideIn(m)!.spots.map((s) => s.direction)).toEqual([DropDirection.Top, DropDirection.Bottom, DropDirection.Left, DropDirection.Center]);
        expect(pillZone(input(m, spot(m, DropDirection.Left), { blockId: "b" }))).toMatchObject({ kind: "place", side: "left" });
    });

    it("a header-only pane (minimized, collapsed) has no body zone", () => {
        const m = twoPanes();
        expect(pillZone(input(m, offSpots(m), { hasBody: false }))).toEqual({ kind: "join" });
    });

    it("a pane the layout has no rectangle for joins, as before", () => {
        const m = twoPanes();
        expect(pillZone(input(m, offSpots(m), { nodeId: "gone" }))).toEqual({ kind: "join" });
    });
});

describe("the placement ghost", () => {
    it("is the split of the hovered pane, with the pill as a node that is in no tree", () => {
        const m = twoPanes();
        const treeReducer = vi.spyOn(m, "treeReducer");
        const i = input(m, offSpots(m));

        showPillGhost(m, i, { kind: "place", direction: DropDirection.Left, side: "left" });

        expect(treeReducer).toHaveBeenCalledTimes(1);
        expect(treeReducer).toHaveBeenCalledWith({
            type: LayoutTreeActionType.SetPendingAction,
            action: expect.objectContaining({
                type: LayoutTreeActionType.SplitHorizontal,
                targetNodeId: "p1",
                newNode: pillStandIn("x"),
                position: "before",
                sizeFraction: 0.5,
            }),
        });
    });

    it("clears the pending action", () => {
        const m = twoPanes();
        const treeReducer = vi.spyOn(m, "treeReducer");
        clearPillGhost(m);
        expect(treeReducer).toHaveBeenCalledWith({ type: LayoutTreeActionType.ClearPendingAction });
    });
});

describe("showPillGhost over a lone pane", () => {
    it("points a root that still has the other axis the way the split goes, so the ghost is drawn along it", () => {
        const m = twoPanes();
        m.treeState.rootNode = leaf("p1", "a");
        m.updateTree();
        const treeReducer = vi.spyOn(m, "treeReducer");
        const root = m.treeState.rootNode;
        const pointer = offSpots(m);

        showPillGhost(m, input(m, pointer, { ownPane: true }), { kind: "place", direction: DropDirection.Left, side: "left" });
        expect(root.flexDirection).toBe(FlexDirection.Row);
        expect(treeReducer).toHaveBeenCalledTimes(1);

        showPillGhost(m, input(m, pointer, { ownPane: true }), { kind: "place", direction: DropDirection.Bottom, side: "down" });
        expect(root.flexDirection).toBe(FlexDirection.Column);
    });
});
