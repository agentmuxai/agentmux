// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A minimized pane (a chip) moves and swaps like any pane and stays a chip; a strip of chips
// does not move or swap; only a chip may go into a chip or a strip of them.
// SPEC_DRAG_MINIMIZED_PANES_2026_10_20.md §3 B. srv's side: crates/srv/src/backend/layout/intent/tests.rs.

import { describe, expect, it } from "vitest";
import { newLayoutNode } from "../lib/layoutNode";
import { computeMoveNode, moveIsLockedByMinimize, moveNode, swapNode } from "../lib/layoutTree";
import {
    DropDirection,
    FlexDirection,
    LayoutTreeActionType,
    type LayoutNode,
    type LayoutTreeMoveNodeAction,
    type LayoutTreeSwapNodeAction,
} from "../lib/types";
import { newLayoutTreeState } from "./model";

const pane = (id: string, minimized = false) => {
    const n = newLayoutNode(FlexDirection.Row, 10, undefined, { blockId: id });
    n.id = id;
    if (minimized) n.minimized = true;
    return n;
};
const branch = (id: string, dir: FlexDirection, children: LayoutNode[]) => {
    const n = newLayoutNode(dir, 10, children);
    n.id = id;
    return n;
};
const ids = (n: LayoutNode) => (n.children ?? []).map((c) => c.id);

describe("moving and swapping a minimized pane", () => {
    it("a chip moves to another parent and stays a chip", () => {
        const col = branch("col", FlexDirection.Column, [pane("b"), pane("c")]);
        const root = branch("r", FlexDirection.Row, [pane("a"), pane("m", true), col]);
        const state = newLayoutTreeState(root);
        const m = root.children![1];
        moveNode(state, { type: LayoutTreeActionType.Move, node: m, parentId: "col", index: 1 } as LayoutTreeMoveNodeAction);
        expect(ids(col)).toEqual(["b", "m", "c"]);
        expect(col.children![1].minimized).toBe(true);
        expect(ids(root)).toEqual(["a", "col"]);
    });

    it("only a chip may go into a strip of chips", () => {
        const strip = () => branch("col", FlexDirection.Column, [pane("m1", true), pane("m2", true)]);
        expect(moveIsLockedByMinimize(pane("a"), strip())).toBe(true);
        expect(moveIsLockedByMinimize(pane("m3", true), strip())).toBe(false);
        expect(moveIsLockedByMinimize(pane("a"), pane("m", true)), "an expanded pane into a chip").toBe(true);
        expect(moveIsLockedByMinimize(strip(), pane("x")), "a strip of chips does not move").toBe(true);
        expect(moveIsLockedByMinimize(pane("a"), branch("c2", FlexDirection.Column, [pane("x")]))).toBe(false);
    });

    it("an expanded pane is not moved into a strip of chips, and no indicator is offered for it", () => {
        const col = branch("col", FlexDirection.Column, [pane("m1", true), pane("m2", true)]);
        const root = branch("r", FlexDirection.Row, [pane("a"), col, pane("c")]);
        const state = newLayoutTreeState(root);
        moveNode(state, { type: LayoutTreeActionType.Move, node: root.children![0], parentId: "col", index: 0 } as LayoutTreeMoveNodeAction);
        expect(ids(col)).toEqual(["m1", "m2"]);
        // Dropped on the bottom edge of a chip in the strip: the move would land in the strip.
        const computed = computeMoveNode(state, { type: LayoutTreeActionType.ComputeMove, nodeId: "m2", nodeToMoveId: "a", direction: DropDirection.Bottom });
        expect(computed).toBeUndefined();
    });

    it("a chip swaps with an expanded pane, and each keeps its state", () => {
        const root = branch("r", FlexDirection.Row, [pane("a"), pane("m", true), pane("c")]);
        const state = newLayoutTreeState(root);
        swapNode(state, { type: LayoutTreeActionType.Swap, node1Id: "m", node2Id: "c" } as LayoutTreeSwapNodeAction);
        expect(ids(root)).toEqual(["a", "c", "m"]);
        expect(root.children![2].minimized).toBe(true);
        expect(root.children![1].minimized).toBeUndefined();
    });

    it("a strip of chips does not swap, and a centre drop on it offers nothing", () => {
        const col = branch("col", FlexDirection.Column, [pane("m1", true), pane("m2", true)]);
        const root = branch("r", FlexDirection.Row, [pane("a"), col, pane("c")]);
        const state = newLayoutTreeState(root);
        swapNode(state, { type: LayoutTreeActionType.Swap, node1Id: "col", node2Id: "c" } as LayoutTreeSwapNodeAction);
        expect(ids(root)).toEqual(["a", "col", "c"]);
        const computed = computeMoveNode(state, { type: LayoutTreeActionType.ComputeMove, nodeId: "col", nodeToMoveId: "a", direction: DropDirection.Center });
        expect(computed).toBeUndefined();
    });
});
