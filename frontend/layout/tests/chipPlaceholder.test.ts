// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A dragged chip's drop placeholder: the chip's size, centred on the seam it is inserted at,
// flush inside the edge at the first or last place. SPEC_CHIP_DROP_PLACEHOLDER_ON_THE_SEAM_2026_10_21.md.

import { describe, expect, it } from "vitest";
import { chipOverChipDirection, chipPlaceholderRect } from "../lib/chipPlaceholder";
import { newLayoutNode } from "../lib/layoutNode";
import { computeMoveNode } from "../lib/layoutTree";
import {
    DropDirection,
    FlexDirection,
    LayoutTreeActionType,
    type LayoutNode,
    type LayoutTreeMoveNodeAction,
} from "../lib/types";
import { newLayoutTreeState } from "./model";

const chip = { width: 600, height: 36 };
// A column of three chips, 36 px each.
const chipAt = (i: number): Dimensions => ({ top: i * 36, left: 0, width: 600, height: 36 });

describe("chipPlaceholderRect", () => {
    it("between two chips, straddles the seam instead of covering the chip", () => {
        const rect = chipPlaceholderRect(chipAt(0), chip, FlexDirection.Column, true, false);
        expect(rect).toEqual({ top: 18, left: 0, width: 600, height: 36 });
    });

    it("at the first place, is flush inside the top edge", () => {
        const rect = chipPlaceholderRect(chipAt(0), chip, FlexDirection.Column, false, true);
        expect(rect).toEqual({ top: 0, left: 0, width: 600, height: 36 });
    });

    it("at the last place, is flush inside the bottom edge", () => {
        const rect = chipPlaceholderRect(chipAt(2), chip, FlexDirection.Column, true, true);
        expect(rect).toEqual({ top: 72, left: 0, width: 600, height: 36 });
    });

    it("after an expanded pane with a pane below, is centred on the seam", () => {
        const pane = { top: 0, left: 0, width: 600, height: 400 };
        const rect = chipPlaceholderRect(pane, chip, FlexDirection.Column, true, false);
        expect(rect).toEqual({ top: 382, left: 0, width: 600, height: 36 });
    });

    it("in a row, straddles the vertical seam with the chip's width", () => {
        const pane = { top: 10, left: 0, width: 300, height: 500 };
        const rect = chipPlaceholderRect(pane, { width: 200, height: 36 }, FlexDirection.Row, true, false);
        expect(rect).toEqual({ top: 10, left: 200, width: 200, height: 36 });
    });

    it("is never bigger than the pane it is placed against", () => {
        const small = { top: 0, left: 0, width: 100, height: 20 };
        const rect = chipPlaceholderRect(small, chip, FlexDirection.Row, false, true);
        expect(rect).toEqual({ top: 0, left: 0, width: 100, height: 20 });
    });
});

// A chip dropped on another chip goes to the nearer seam and the placeholder straddles it, not the
// centre swap or the end-wedge split that covers the chip. SPEC_CHIP_OVER_CHIP_DROP_2026_10_24.md.
// A leaf lays out against its parent, as balanceNode leaves a real tree.
const node = (id: string, minimized = false, dir = FlexDirection.Row) => {
    const n = newLayoutNode(dir, 10, undefined, { blockId: id });
    n.id = id;
    if (minimized) n.minimized = true;
    return n;
};
const branch = (id: string, dir: FlexDirection, children: LayoutNode[]) => {
    const n = newLayoutNode(dir, 10, children);
    n.id = id;
    return n;
};

describe("a chip dropped on another chip", () => {
    // A column strip m1 m2 m3 (36 px each, x 100..700) beside an expanded pane, and a chip d to drag.
    const build = (dir: FlexDirection) => {
        const leaf = dir === FlexDirection.Column ? FlexDirection.Row : FlexDirection.Column;
        const strip = branch("strip", dir, [node("m1", true, leaf), node("m2", true, leaf), node("m3", true, leaf)]);
        const root = branch("r", dir === FlexDirection.Column ? FlexDirection.Row : FlexDirection.Column, [
            node("a"),
            strip,
            node("d", true),
        ]);
        return { root, strip };
    };
    const colRect = (i: number): Dimensions => ({ top: 100 + i * 36, left: 100, width: 600, height: 36 });
    const rowRect = (i: number): Dimensions => ({ top: 100, left: 100 + i * 200, width: 200, height: 36 });

    /** What the drop shows, as the model computes it: the direction, the move, then the placeholder. */
    const drop = (dir: FlexDirection, targetId: string, at: { x: number; y: number }) => {
        const { root, strip } = build(dir);
        const rects = dir === FlexDirection.Column ? colRect : rowRect;
        const idx = strip.children!.findIndex((c) => c.id === targetId);
        const direction = chipOverChipDirection(root, targetId, "d", rects(idx), at);
        if (direction === undefined) return { direction };
        const move = computeMoveNode(newLayoutTreeState(root), {
            type: LayoutTreeActionType.ComputeMove,
            nodeId: targetId,
            nodeToMoveId: "d",
            direction,
        }) as LayoutTreeMoveNodeAction;
        if (!move) return { direction, move };
        const targetIndex = Math.max(0, Math.min(move.index - 1, strip.children!.length - 1));
        const after = move.index > targetIndex;
        const outerEdge = !after || move.index >= strip.children!.length;
        const rect = chipPlaceholderRect(
            rects(targetIndex),
            { width: dir === FlexDirection.Column ? 600 : 200, height: 36 },
            dir,
            after,
            outerEdge
        );
        return { direction, move, rect };
    };

    it("in a column, the upper half of the middle chip puts the placeholder on the seam above it, wherever the cursor is", () => {
        // The centre of the chip (generic: a swap), its ends (generic: a split of the chip) and its middle third.
        for (const x of [101, 250, 400, 650, 699]) {
            const r = drop(FlexDirection.Column, "m2", { x, y: 140 });
            expect(r.direction, `x=${x}`).toBe(DropDirection.Top);
            expect(r.move).toMatchObject({ parentId: "strip", index: 1 });
            // m1 spans 100..136: the seam is 136, the 36 px placeholder 118..154.
            expect(r.rect).toEqual({ top: 118, left: 100, width: 600, height: 36 });
        }
    });

    it("in a column, the lower half puts it on the seam below the chip", () => {
        for (const x of [101, 400, 699]) {
            const r = drop(FlexDirection.Column, "m2", { x, y: 160 });
            expect(r.direction, `x=${x}`).toBe(DropDirection.Bottom);
            expect(r.move).toMatchObject({ parentId: "strip", index: 2 });
            expect(r.rect).toEqual({ top: 154, left: 100, width: 600, height: 36 });
        }
    });

    it("at the strip's ends it is flush inside the edge", () => {
        expect(drop(FlexDirection.Column, "m1", { x: 400, y: 110 }).rect).toEqual({
            top: 100,
            left: 100,
            width: 600,
            height: 36,
        });
        expect(drop(FlexDirection.Column, "m3", { x: 400, y: 200 }).rect).toEqual({
            top: 172,
            left: 100,
            width: 600,
            height: 36,
        });
    });

    it("in a row, the nearer vertical seam is chosen by the horizontal half", () => {
        const left = drop(FlexDirection.Row, "m2", { x: 320, y: 118 });
        expect(left.direction).toBe(DropDirection.Left);
        expect(left.rect).toEqual({ top: 100, left: 200, width: 200, height: 36 });
        const right = drop(FlexDirection.Row, "m2", { x: 450, y: 118 });
        expect(right.direction).toBe(DropDirection.Right);
        expect(right.rect).toEqual({ top: 100, left: 400, width: 200, height: 36 });
    });

    it("applies only chip over chip, inside the chip, in this tree", () => {
        const { root } = build(FlexDirection.Column);
        const r = colRect(1);
        expect(chipOverChipDirection(root, "a", "d", r, { x: 400, y: 140 }), "target not a chip").toBeUndefined();
        expect(chipOverChipDirection(root, "m2", "a", r, { x: 400, y: 140 }), "dragged not a chip").toBeUndefined();
        expect(chipOverChipDirection(root, "m2", "d", r, { x: 400, y: 10 }), "outside the chip").toBeUndefined();
        expect(
            chipOverChipDirection(root, "m2", "elsewhere", r, { x: 400, y: 140 }),
            "dragged from another tab"
        ).toBeUndefined();
    });
});
