// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { keyboardResizeOperations } from "./layoutKeyboard";
import { FlexDirection, NavigateDirection, type LayoutNode } from "./types";

function leaf(id: string, size = 1): LayoutNode {
    return { id, size, flexDirection: FlexDirection.Row, data: { blockId: id } };
}
function split(id: string, dir: FlexDirection, children: LayoutNode[], size = 1): LayoutNode {
    return { id, size, flexDirection: dir, children };
}

// [ A | [ B / C ] ]
const tree = split("root", FlexDirection.Row, [
    leaf("A"),
    split("right", FlexDirection.Column, [leaf("B"), leaf("C")]),
]);

describe("keyboardResizeOperations", () => {
    it("Right grows a pane that has a neighbour to its right", () => {
        expect(keyboardResizeOperations(tree, "A", NavigateDirection.Right, 0.1)).toEqual([
            { nodeId: "A", size: 1.2 },
            { nodeId: "right", size: 0.8 },
        ]);
    });

    it("Right on the rightmost pane moves its left border, shrinking it", () => {
        const ops = keyboardResizeOperations(tree, "B", NavigateDirection.Right, 0.1);
        expect(ops).toEqual([
            { nodeId: "A", size: 1.2 },
            { nodeId: "right", size: 0.8 },
        ]);
    });

    it("Down/Up use the nearest column split", () => {
        expect(keyboardResizeOperations(tree, "B", NavigateDirection.Down, 0.1)).toEqual([
            { nodeId: "B", size: 1.2 },
            { nodeId: "C", size: 0.8 },
        ]);
        expect(keyboardResizeOperations(tree, "C", NavigateDirection.Up, 0.1)).toEqual([
            { nodeId: "B", size: 0.8 },
            { nodeId: "C", size: 1.2 },
        ]);
    });

    it("returns null when there is no split on that axis", () => {
        expect(keyboardResizeOperations(tree, "A", NavigateDirection.Down)).toBeNull();
    });

    it("stops at the minimum share instead of collapsing a pane", () => {
        const lopsided = split("root", FlexDirection.Row, [leaf("A", 1.85), leaf("B", 0.15)]);
        expect(keyboardResizeOperations(lopsided, "A", NavigateDirection.Right, 0.1)).toEqual([]);
    });
});
