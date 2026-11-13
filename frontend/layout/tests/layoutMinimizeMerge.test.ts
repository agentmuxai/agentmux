// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { findExpandedMergeTarget, findMinimizeMergeTarget } from "../lib/layoutMinimizeMerge";
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

    it("does not reach past an expanded pane in a Column, where chips lie along the column", () => {
        const [a, x, b, y, c] = [leaf("a", true), leaf("x"), leaf("b"), leaf("y"), leaf("c", true)];
        expect(targetOf(split(FlexDirection.Column, a, x, b, y, c), b)).toBeUndefined();
    });

    it("in a Row, takes a chip that stacks over the same expanded pane although that pane sits between them", () => {
        // a docks right onto x; once b is minimized it docks right onto y: not the same pane, no target.
        const [a, x, b, y, c] = [leaf("a", true), leaf("x"), leaf("b"), leaf("y"), leaf("c", true)];
        expect(targetOf(split(FlexDirection.Row, a, x, b, y, c), b)).toBe("c");
        // The operator's case: chip, expanded pane, then the pane minimized: both stack over the middle one.
        const [p, q, r] = [leaf("p", true), leaf("q"), leaf("r")];
        expect(targetOf(split(FlexDirection.Row, p, q, r), r)).toBe("p");
        // And with the chip on the right of the pane that minimizes.
        const [s1, s2, s3] = [leaf("s1"), leaf("s2"), leaf("s3", true)];
        expect(targetOf(split(FlexDirection.Row, s1, s2, s3), s1)).toBe("s3");
    });

    it("in a Row, a chip that docks onto a different expanded pane is not a target", () => {
        const [a, x, b, y] = [leaf("a", true), leaf("x"), leaf("b"), leaf("y")];
        // a docks onto x; b, once minimized, docks onto y.
        expect(targetOf(split(FlexDirection.Row, a, x, b, y), b)).toBeUndefined();
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

describe("findExpandedMergeTarget", () => {
    const expandedOf = (root: LayoutNode, node: LayoutNode) => findExpandedMergeTarget(root, node.id)?.data?.blockId;

    it("takes the nearest expanded sibling after the pane, else before, in a Row and in a Column", () => {
        for (const direction of [FlexDirection.Row, FlexDirection.Column]) {
            const [a, b, c] = [leaf("a"), leaf("b"), leaf("c")];
            const root = split(direction, a, b, c);
            expect(expandedOf(root, a)).toBe("b");
            expect(expandedOf(root, b)).toBe("c");
            expect(expandedOf(root, c)).toBe("b");
        }
    });

    it("steps over a minimized pane lying between the pane and the expanded one", () => {
        const [a, m, c] = [leaf("a"), leaf("m", true), leaf("c")];
        expect(expandedOf(split(FlexDirection.Row, a, m, c), a)).toBe("c");
        const [d, n, e] = [leaf("d"), leaf("n", true), leaf("e")];
        expect(expandedOf(split(FlexDirection.Column, d, n, e), e)).toBe("d");
    });

    it("takes the leaf of a split sibling nearest to the pane", () => {
        // Along the split's own axis: the first child when coming from before it, the last from after.
        const [a, x, y] = [leaf("a"), leaf("x"), leaf("y")];
        expect(expandedOf(split(FlexDirection.Row, a, split(FlexDirection.Row, x, y)), a)).toBe("x");
        const [x2, y2, a2] = [leaf("x2"), leaf("y2"), leaf("a2")];
        expect(expandedOf(split(FlexDirection.Row, split(FlexDirection.Row, x2, y2), a2), a2)).toBe("y2");
        // Across it every child touches the near edge: the first (topmost) one.
        const [a3, x3, y3] = [leaf("a3"), leaf("x3"), leaf("y3")];
        expect(expandedOf(split(FlexDirection.Row, a3, split(FlexDirection.Column, x3, y3)), a3)).toBe("x3");
    });

    it("skips a split sibling whose leaves are all minimized", () => {
        const [a, x, y, z] = [leaf("a"), leaf("x", true), leaf("y", true), leaf("z")];
        expect(expandedOf(split(FlexDirection.Row, a, split(FlexDirection.Column, x, y), z), a)).toBe("z");
    });

    it("has no target when every other pane is minimized, or the pane is alone", () => {
        const [a, b, c] = [leaf("a"), leaf("b", true), leaf("c", true)];
        expect(expandedOf(split(FlexDirection.Row, a, b, c), a)).toBeUndefined();
        const only = leaf("only");
        expect(expandedOf(split(FlexDirection.Row, only), only)).toBeUndefined();
    });
});
