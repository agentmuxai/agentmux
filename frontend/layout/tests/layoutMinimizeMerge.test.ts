// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { findMinimizeMergeTarget } from "../lib/layoutMinimizeMerge";
import { newLayoutNode } from "../lib/layoutNode";
import { FlexDirection, type LayoutNode } from "../lib/types";

const leaf = (blockId: string, minimized = false): LayoutNode => {
    const node = newLayoutNode(undefined, 100, undefined, { blockId });
    if (minimized) node.minimized = true;
    return node;
};
const split = (direction: FlexDirection, ...children: LayoutNode[]) => newLayoutNode(direction, 100, children);
const targetOf = (root: LayoutNode, node: LayoutNode) => findMinimizeMergeTarget(root, node.id)?.data?.blockId;

describe("findMinimizeMergeTarget", () => {
    it("takes the nearest minimized sibling before the pane, in a Row and in a Column", () => {
        for (const direction of [FlexDirection.Row, FlexDirection.Column]) {
            const [a, b, c, d] = [leaf("a", true), leaf("b", true), leaf("c"), leaf("d", true)];
            const root = split(direction, a, b, c, d);
            expect(targetOf(root, c)).toBe("b");
        }
    });

    it("falls back to the nearest minimized sibling after it when none is before", () => {
        const [a, b, c] = [leaf("a"), leaf("b"), leaf("c", true)];
        expect(targetOf(split(FlexDirection.Row, a, b, c), b)).toBe("c");
        const [d, e, f] = [leaf("d"), leaf("e", true), leaf("f", true)];
        expect(targetOf(split(FlexDirection.Column, d, e, f), d)).toBe("e");
    });

    it("prefers the side before when both sides have one", () => {
        const [a, b, c] = [leaf("a", true), leaf("b"), leaf("c", true)];
        expect(targetOf(split(FlexDirection.Row, a, b, c), b)).toBe("a");
    });

    it("does not reach past an expanded pane", () => {
        const [a, x, b, y, c] = [leaf("a", true), leaf("x"), leaf("b"), leaf("y"), leaf("c", true)];
        expect(targetOf(split(FlexDirection.Row, a, x, b, y, c), b)).toBeUndefined();
    });

    it("steps over a fully-collapsed branch, which is a group and never a target", () => {
        const group = split(FlexDirection.Column, leaf("g1", true), leaf("g2", true));
        const [a, b] = [leaf("a", true), leaf("b")];
        expect(targetOf(split(FlexDirection.Row, a, group, b), b)).toBe("a");
        const lone = leaf("c");
        expect(targetOf(split(FlexDirection.Row, group, lone), lone)).toBeUndefined();
    });

    it("never merges across splits", () => {
        const [a, b] = [leaf("a", true), leaf("b")];
        const root = split(
            FlexDirection.Row,
            split(FlexDirection.Column, a, leaf("a2")),
            split(FlexDirection.Column, b, leaf("b2"))
        );
        expect(targetOf(root, b)).toBeUndefined();
    });

    it("has no target for a minimized pane, a branch, the root, or an unknown id", () => {
        const [a, b] = [leaf("a", true), leaf("b", true)];
        const root = split(FlexDirection.Row, a, b, leaf("c"));
        expect(targetOf(root, b)).toBeUndefined();
        expect(findMinimizeMergeTarget(root, root.id)).toBeUndefined();
        expect(findMinimizeMergeTarget(root, "nope")).toBeUndefined();
        expect(findMinimizeMergeTarget(undefined, "a")).toBeUndefined();
        const only = leaf("only");
        expect(findMinimizeMergeTarget(only, only.id)).toBeUndefined();
    });
});
