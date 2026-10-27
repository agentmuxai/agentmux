// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A chip docked onto a pane of its Row grows in that pane's column when it is restored, not in a
// column of its own where it was minimized from (layoutRestore.ts). srv does the same
// (crates/srv/src/backend/layout/intent.rs); layoutIntentVectors.test.ts checks the two agree.

import { describe, expect, it } from "vitest";
import { withDerivedNodeIds } from "../lib/layoutNode";
import { restoreNode } from "../lib/layoutTree";
import { FlexDirection, type LayoutNode, type LayoutTreeState } from "../lib/types";

const ACTION = "a0000001-0000-4000-8000-00000000abcd";
const GROUP = "00000001-0000-4000-8000-00000000abcd";

const leaf = (id: string, size = 10): LayoutNode => ({
    id,
    flexDirection: FlexDirection.Row,
    size,
    data: { blockId: `b-${id}` } as LayoutNode["data"],
});
const chip = (id: string): LayoutNode => ({ ...leaf(id), minimized: true });
const branch = (id: string, flexDirection: FlexDirection, children: LayoutNode[]): LayoutNode => ({
    id,
    flexDirection,
    size: 10,
    children,
});
const row = (...children: LayoutNode[]) => branch("r", FlexDirection.Row, children);
const column = (id: string, ...children: LayoutNode[]) => branch(id, FlexDirection.Column, children);

function restore(root: LayoutNode, nodeId: string): LayoutNode {
    const state = { rootNode: root } as LayoutTreeState;
    withDerivedNodeIds(ACTION, () => restoreNode(state, { type: "restore", nodeId } as never));
    return state.rootNode;
}

const ids = (n: LayoutNode) => (n.children ?? []).map((c) => c.id);
const find = (n: LayoutNode, id: string): LayoutNode | undefined =>
    n.id === id ? n : n.children?.map((c) => find(c, id)).find(Boolean);

describe("restoring a chip docked onto a pane of its Row", () => {
    it("grows it in that pane's column, not in a column of its own", () => {
        const root = restore(row(leaf("a"), chip("b"), leaf("c")), "b");
        expect(ids(root)).toEqual(["a", GROUP]);
        const group = find(root, GROUP)!;
        expect(group.flexDirection).toBe(FlexDirection.Column);
        expect(ids(group)).toEqual(["b", "c"]);
        expect(group.size).toBe(10);
        expect(find(root, "b")!.minimized).toBeUndefined();
    });

    it("takes its place in the stack of chips above the pane", () => {
        const root = restore(row(leaf("a"), chip("m1"), chip("m2"), leaf("c")), "m2");
        expect(ids(root)).toEqual(["a", GROUP]);
        expect(ids(find(root, GROUP)!)).toEqual(["m1", "m2", "c"]);
        expect(find(root, "m1")!.minimized).toBe(true);
        expect(find(root, "m2")!.minimized).toBeUndefined();
    });

    it("joins the column the pane is already in, with a share like its panes", () => {
        const col = column("col", leaf("x", 6), leaf("y", 10));
        const root = restore(row(leaf("a"), chip("m"), col), "m");
        expect(ids(root)).toEqual(["a", "col"]);
        expect(ids(find(root, "col")!)).toEqual(["m", "x", "y"]);
        expect(find(root, "m")!.size).toBe(8);
    });

    it("docks left when no pane is to its right", () => {
        const root = restore(row(leaf("a"), leaf("b"), chip("m")), "m");
        expect(ids(root)).toEqual(["a", GROUP]);
        expect(ids(find(root, GROUP)!)).toEqual(["m", "b"]);
    });

    it("follows the stack when it is one of a column of chips", () => {
        const root = restore(row(leaf("a"), column("chips", chip("m1"), chip("m2")), leaf("c")), "m1");
        expect(ids(root)).toEqual(["a", GROUP]);
        expect(ids(find(root, GROUP)!)).toEqual(["m1", "m2", "c"]);
        expect(find(root, "m2")!.minimized).toBe(true);
    });

    it("moves the last chip of the leftmost slot into the next pane's column", () => {
        const root = restore(row(chip("x"), leaf("a"), leaf("b")), "x");
        expect(ids(root)).toEqual([GROUP, "b"]);
        expect(ids(find(root, GROUP)!)).toEqual(["x", "a"]);
    });
});

describe("restoring a chip that shows in its own column", () => {
    it("grows a chip in a stack in place", () => {
        const col = column("col", leaf("x"), chip("m"), leaf("y"));
        const root = restore(row(leaf("a"), col), "m");
        expect(ids(find(root, "col")!)).toEqual(["x", "m", "y"]);
        expect(find(root, "m")!.minimized).toBeUndefined();
    });

    it("keeps the span of a pane under two columns", () => {
        const top = row(leaf("a"), leaf("b"));
        const root = restore(branch("root", FlexDirection.Column, [{ ...top, id: "top" }, chip("wide")]), "wide");
        expect(ids(root)).toEqual(["top", "wide"]);
        expect(find(root, "wide")!.minimized).toBeUndefined();
    });

    it("leaves the tree alone when every other pane in the Row is a chip too", () => {
        const root = restore(branch("r", FlexDirection.Row, [chip("p"), chip("q")]), "p");
        expect(ids(root)).toEqual(["p", "q"]);
    });
});
