// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { allBlockIds, blocksInLayoutOrder } from "@/layout/lib/stackOrder";
import type { LayoutNode } from "@/layout/lib/types";

const leaf = (id: string, stack: string[]): LayoutNode =>
    ({
        id,
        data: { blockId: stack[0], blockStack: stack.length > 1 ? stack : undefined, activeBlockId: stack[0] },
    }) as unknown as LayoutNode;

/** root: [ p1 [a1 a2], branch [ p2 [b1], p3 [c1 c2 c3] ], p4 [d1] ] */
const tree = {
    id: "root",
    children: [
        leaf("p1", ["a1", "a2"]),
        { id: "branch", children: [leaf("p2", ["b1"]), leaf("p3", ["c1", "c2", "c3"])] },
        leaf("p4", ["d1"]),
    ],
} as unknown as LayoutNode;

describe("blocksInLayoutOrder", () => {
    it("orders by pane in tree order, then by strip order, whatever order they are asked for", () => {
        expect(blocksInLayoutOrder(tree, ["d1", "c3", "a2", "c1", "b1"])).toEqual(["a2", "b1", "c1", "c3", "d1"]);
    });

    it("drops ids that are in no pane, and lists a repeated id once", () => {
        expect(blocksInLayoutOrder(tree, ["ghost", "b1", "b1"])).toEqual(["b1"]);
    });

    it("is empty for no tree or nothing wanted", () => {
        expect(blocksInLayoutOrder(undefined, ["a1"])).toEqual([]);
        expect(blocksInLayoutOrder(tree, [])).toEqual([]);
    });
});

describe("allBlockIds", () => {
    it("lists every tab of every pane, dormant ones included", () => {
        expect([...allBlockIds(tree)].sort()).toEqual(["a1", "a2", "b1", "c1", "c2", "c3", "d1"]);
        expect(allBlockIds(undefined).size).toBe(0);
    });
});
