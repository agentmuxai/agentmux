// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** One block, one pane: a placement of a block the layout already holds shows that pane, and a tree that holds one twice is healed. */

import { describe, expect, it, vi } from "vitest";
import { alreadyHeld, duplicateBlockIds, healDuplicateBlocks, leafHolding } from "@/layout/lib/layoutDuplicates";
import { validateLayoutInvariants } from "@/layout/lib/layoutInvariants";
import { insertNode, insertNodeAtIndex } from "@/layout/lib/layoutTree";
import { FlexDirection, type LayoutNode, type LayoutTreeState } from "@/layout/lib/types";

const leaf = (id: string, blockId: string, stack?: string[]): LayoutNode =>
    ({ id, size: 10, data: { blockId, ...(stack ? { blockStack: stack, activeBlockId: stack[0] } : {}) } }) as LayoutNode;
const row = (id: string, ...children: LayoutNode[]): LayoutNode =>
    ({ id, size: 10, flexDirection: FlexDirection.Row, children }) as LayoutNode;
const state = (rootNode: LayoutNode): LayoutTreeState => ({ rootNode, focusedNodeId: "x" }) as LayoutTreeState;
const ids = (root: LayoutNode): string[] => {
    const out: string[] = [];
    (function walk(n: LayoutNode) {
        if (n.children?.length) n.children.forEach(walk);
        else out.push(n.id);
    })(root);
    return out;
};

describe("a block held in two places", () => {
    // The operator's tab group, as the layout showed it: Agent2 and Agent2 #5 as the tab strip of two leaves.
    const twice = () => row("r", leaf("left", "a", ["a", "b"]), leaf("right", "b", ["b", "a"]), leaf("other", "c"));

    it("is found, whether it sits in two strips, two leaves, or twice in one strip", () => {
        expect(duplicateBlockIds(twice()).sort()).toEqual(["a", "b"]);
        expect(duplicateBlockIds(row("r", leaf("1", "a"), leaf("2", "a")))).toEqual(["a"]);
        expect(duplicateBlockIds(row("r", leaf("1", "a", ["a", "a"]), leaf("2", "b")))).toEqual(["a"]);
        expect(duplicateBlockIds(row("r", leaf("1", "a"), leaf("2", "b")))).toEqual([]);
    });

    it("is reported by the layout doctor", () => {
        expect(validateLayoutInvariants(twice()).map((v) => v.code)).toContain("DUPLICATE_BLOCK");
        expect(validateLayoutInvariants(row("r", leaf("1", "a"), leaf("2", "b"))).map((v) => v.code)).not.toContain("DUPLICATE_BLOCK");
    });

    it("is healed to its first holder: the later strip loses the repeats and the leaf stays while it has a tab", () => {
        const tree = row("r", leaf("left", "a", ["a", "b"]), leaf("right", "b", ["b", "c"]), leaf("other", "d"));
        expect(healDuplicateBlocks(tree)).toEqual(["b"]);
        expect(leafHolding(tree, "b")?.id).toBe("left");
        expect(leafHolding(tree, "c")?.id).toBe("right");
        // One tab left: the leaf is a plain pane of it, not a one-tab strip.
        const right = leafHolding(tree, "c")!;
        expect(right.data).toEqual({ blockId: "c", blockStack: undefined });
        expect(duplicateBlockIds(tree)).toEqual([]);
    });

    it("is healed by removing a leaf that holds nothing but repeats, and keeps the active tab on a member that is left", () => {
        const tree = row("r", leaf("left", "a", ["a", "b"]), leaf("right", "b"), leaf("third", "c", ["c", "a"]));
        expect(healDuplicateBlocks(tree).sort()).toEqual(["a", "b"]);
        expect(ids(tree)).toEqual(["left", "third"]);
        expect(leafHolding(tree, "c")?.data?.blockId).toBe("c");
        // A strip whose active tab was a repeat moves it to the first member that is left.
        const t2 = row("r", leaf("one", "a"), leaf("two", "a", ["a", "b", "c"]));
        t2.children![1].data!.activeBlockId = "a";
        healDuplicateBlocks(t2);
        expect(t2.children![1].data).toMatchObject({ blockStack: ["b", "c"], activeBlockId: "b", blockId: "b" });
        // A surviving active tab that is not first stays the shown one: blockId and activeBlockId still agree.
        const t3 = row("r", leaf("one", "a"), leaf("two", "a", ["a", "b", "c"]));
        t3.children![1].data!.activeBlockId = "c";
        t3.children![1].data!.blockId = "c";
        healDuplicateBlocks(t3);
        expect(t3.children![1].data).toMatchObject({ blockStack: ["b", "c"], activeBlockId: "c", blockId: "c" });
    });

    it("drops a block repeated inside one strip", () => {
        const tree = row("r", leaf("one", "a", ["a", "b", "a"]), leaf("two", "c"));
        expect(healDuplicateBlocks(tree)).toEqual(["a"]);
        expect(leafHolding(tree, "b")?.data).toMatchObject({ blockStack: ["a", "b"], blockId: "a" });
        expect(duplicateBlockIds(tree)).toEqual([]);
    });

    it("leaves a healthy tree untouched", () => {
        const tree = row("r", leaf("1", "a"), leaf("2", "b", ["b", "c"]));
        const before = JSON.stringify(tree);
        expect(healDuplicateBlocks(tree)).toEqual([]);
        expect(JSON.stringify(tree)).toBe(before);
    });
});

describe("placing a block the layout already holds", () => {
    const warn = () => vi.spyOn(console, "warn").mockImplementation(() => {});

    it("insertNode shows the existing pane instead of adding a second one", () => {
        warn();
        const s = state(row("r", leaf("p1", "a"), leaf("p2", "b", ["b", "c"])));
        insertNode(s, { type: "insert", node: leaf("new", "a"), focused: true } as any);
        expect(ids(s.rootNode!)).toEqual(["p1", "p2"]);
        expect(s.focusedNodeId).toBe("p1");
        // A block that is a background tab of a strip counts as held too, and focus moves to that strip.
        insertNode(s, { type: "insert", node: leaf("new2", "c"), focused: true } as any);
        expect(ids(s.rootNode!)).toEqual(["p1", "p2"]);
        expect(s.focusedNodeId).toBe("p2");
    });

    it("leaves focus where it was when the placement did not ask for it", () => {
        warn();
        const s = state(row("r", leaf("p1", "a"), leaf("p2", "b")));
        insertNode(s, { type: "insert", node: leaf("new", "a") } as any);
        expect(s.focusedNodeId).toBe("x");
    });

    it("insertNodeAtIndex is refused the same way", () => {
        warn();
        const s = state(row("r", leaf("p1", "a"), leaf("p2", "b")));
        insertNodeAtIndex(s, { type: "insertatindex", node: leaf("new", "b"), indexArr: [0], focused: true } as any);
        expect(ids(s.rootNode!)).toEqual(["p1", "p2"]);
        expect(s.focusedNodeId).toBe("p2");
    });

    it("places a block the layout does not hold, as before", () => {
        const s = state(row("r", leaf("p1", "a"), leaf("p2", "b")));
        insertNode(s, { type: "insert", node: leaf("new", "z"), focused: true } as any);
        expect(ids(s.rootNode!)).toContain("new");
        expect(s.focusedNodeId).toBe("new");
    });

    it("alreadyHeld is false for a node that carries no block", () => {
        const s = state(row("r", leaf("p1", "a")));
        expect(alreadyHeld(s, { id: "n", size: 1, data: {} } as LayoutNode, true)).toBe(false);
    });
});
