// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

/**
 * Which zone of a pane a dragged tab pill is over: its header and the middle of its body join the
 * pill to the pane's tabs, the rest of the body places it as a pane of its own on that side.
 * docs/specs/SPEC_PANE_TAB_DRAG_BODY_PLACEMENT_AND_EDGE_GLOW_2026_10_29.md §3.1.
 */

import { describe, expect, it, vi } from "vitest";
import type { LayoutModel } from "@/layout/lib/layoutModel";
import { clearPillGhost, pillStandIn, pillZone, showPillGhost, type PillZoneInput } from "@/layout/lib/paneBodyDrop";
import { LayoutTreeActionType } from "@/layout/lib/types";

const RECT = { left: 100, top: 50, width: 500, height: 400 };
const HEADER = { left: 100, top: 50, right: 600, bottom: 83 };

/** A model whose container sits at (10, 20) in the window and whose pane `p1` is RECT in it. */
function fakeModel(opts: { rect?: typeof RECT | undefined } = { rect: RECT }) {
    const treeReducer = vi.fn();
    const model = {
        treeState: { rootNode: { id: "p1", data: { blockId: "a" } } },
        displayContainerRef: { current: { getBoundingClientRect: () => ({ x: 10, y: 20, left: 10, top: 20 }) } },
        additionalProps: () => (opts.rect ? { p1: { rect: opts.rect } } : {}),
        treeReducer,
    } as unknown as LayoutModel;
    return { model, treeReducer };
}

/** The pointer at (x, y) inside RECT's pane, in the window's coordinates. */
const at = (x: number, y: number) => ({ x: x + 10, y: y + 20 });

function input(model: LayoutModel, pointer: { x: number; y: number }, extra: Partial<PillZoneInput> = {}): PillZoneInput {
    return {
        model,
        nodeId: "p1",
        blockId: "b",
        pointer,
        headerRect: HEADER,
        ownPane: false,
        hasBody: true,
        ...extra,
    };
}

describe("pillZone", () => {
    it("the header joins the pill to the pane's tabs, wherever in the header", () => {
        const { model } = fakeModel();
        expect(pillZone(input(model, { x: 120, y: 60 }))).toEqual({ kind: "join" });
        expect(pillZone(input(model, { x: 590, y: 82 }))).toEqual({ kind: "join" });
    });

    it("the middle fifth of the body joins too, and shows no ghost", () => {
        const { model } = fakeModel();
        expect(pillZone(input(model, at(RECT.left + 250, RECT.top + 200)))).toEqual({ kind: "join" });
    });

    it("the rest of the body places the pill on the side the pointer is nearest", () => {
        const { model } = fakeModel();
        // RECT is in the container's coordinates (left 100, top 50); pointer offsets are relative to it.
        const place = (x: number, y: number) => pillZone(input(model, at(x, y)));
        expect(place(RECT.left + 30, RECT.top + 200)).toMatchObject({ kind: "place", side: "left" });
        expect(place(RECT.left + 470, RECT.top + 200)).toMatchObject({ kind: "place", side: "right" });
        expect(place(RECT.left + 250, RECT.top + 110)).toMatchObject({ kind: "place", side: "up" });
        expect(place(RECT.left + 250, RECT.top + 390)).toMatchObject({ kind: "place", side: "down" });
    });

    it("an outer band is the inner half-split, so the ghost is what the drop makes", () => {
        const { model } = fakeModel();
        const zone = pillZone(input(model, at(RECT.left + 5, RECT.top + 200)));
        expect(zone).toMatchObject({ kind: "place", side: "left" });
        // OuterLeft (the pane drag's full-height band) is clamped to Left.
        expect((zone as { direction: number }).direction).toBeLessThan(4);
    });

    it("a pill over its own pane can be placed but never joined to where it already is", () => {
        const { model } = fakeModel();
        expect(pillZone(input(model, { x: 120, y: 60 }, { ownPane: true }))).toEqual({ kind: "none" });
        expect(pillZone(input(model, at(RECT.left + 250, RECT.top + 200), { ownPane: true }))).toEqual({ kind: "none" });
        expect(pillZone(input(model, at(RECT.left + 30, RECT.top + 200), { ownPane: true }))).toMatchObject({
            kind: "place",
            side: "left",
        });
    });

    it("a header-only pane (minimized, collapsed) has no body zone", () => {
        const { model } = fakeModel();
        expect(pillZone(input(model, at(RECT.left + 30, RECT.top + 200), { hasBody: false }))).toEqual({ kind: "join" });
    });

    it("a pane the layout has no rectangle for joins, as before", () => {
        const { model } = fakeModel({ rect: undefined });
        expect(pillZone(input(model, at(RECT.left + 30, RECT.top + 200)))).toEqual({ kind: "join" });
    });
});

describe("the placement ghost", () => {
    it("is the pane drag's ComputeMove, with the pill as a node that is in no tree", () => {
        const { model, treeReducer } = fakeModel();
        const i = input(model, at(RECT.left + 30, RECT.top + 200));
        const zone = pillZone(i);
        if (zone.kind !== "place") throw new Error("expected a place zone");

        showPillGhost(model, i, zone);

        expect(treeReducer).toHaveBeenCalledTimes(1);
        const action = treeReducer.mock.calls[0][0];
        expect(action).toMatchObject({
            type: LayoutTreeActionType.ComputeMove,
            nodeId: "p1",
            nodeToMoveId: "pill:b",
            direction: zone.direction,
        });
        expect(action.nodeToMove).toEqual(pillStandIn("b"));
    });

    it("clears the pending action", () => {
        const { model, treeReducer } = fakeModel();
        clearPillGhost(model);
        expect(treeReducer).toHaveBeenCalledWith({ type: LayoutTreeActionType.ClearPendingAction });
    });
});
