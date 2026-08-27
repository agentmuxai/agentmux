// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { assert, test } from "vitest";
import {
    computeWindowEdgeResizeOps,
    spillInwardBy,
    type EdgeResizeSnapshotNode,
} from "../lib/windowEdgeResize";
import { FlexDirection, ResizeNodeOperation } from "../lib/types";

const MIN = 128;

function sum(values: number[]): number {
    return values.reduce((a, b) => a + b, 0);
}

function leaf(id: string, size: number, width: number, height: number): EdgeResizeSnapshotNode {
    return { id, flexDirection: FlexDirection.Column, size, width, height };
}

function branch(
    id: string,
    flexDirection: FlexDirection,
    size: number,
    width: number,
    height: number,
    children: EdgeResizeSnapshotNode[]
): EdgeResizeSnapshotNode {
    return { id, flexDirection, size, width, height, children };
}

function opsMap(ops: ResizeNodeOperation[]): Map<string, number> {
    return new Map(ops.map((op) => [op.nodeId, op.size]));
}

/** Resolve the pixel size each child of `parent` would get under the new
 *  weights (ops overriding snapshot weights), for a new parent extent —
 *  mirrors computeMainAxisAllocation's `size / (flexTotal / totalPx)`. */
function resolvedPx(parent: EdgeResizeSnapshotNode, ops: ResizeNodeOperation[], newTotalPx: number): number[] {
    const m = opsMap(ops);
    const weights = parent.children!.map((c) => m.get(c.id) ?? c.size);
    const weightSum = sum(weights);
    return weights.map((w) => (w / weightSum) * newTotalPx);
}

// ---------------------------------------------------------------------------
// spillInwardBy — the pure edge-first spill primitive
// ---------------------------------------------------------------------------

test("spillInwardBy - grow routes the entire delta to the edge member (last)", () => {
    assert.deepEqual(spillInwardBy([300, 300, 300], 150, MIN, false), [300, 300, 450]);
});

test("spillInwardBy - grow routes the entire delta to the edge member (first)", () => {
    assert.deepEqual(spillInwardBy([300, 300, 300], 150, MIN, true), [450, 300, 300]);
});

test("spillInwardBy - shrink within the edge member's headroom touches only it", () => {
    assert.deepEqual(spillInwardBy([200, 300, 400], -150, MIN, false), [200, 300, 250]);
});

test("spillInwardBy - shrink past the edge member's floor spills inward, accordion-style", () => {
    // c gives 272 (400 -> 128), remaining 78 comes from b (300 -> 222); a untouched.
    assert.deepEqual(spillInwardBy([200, 300, 400], -350, MIN, false), [200, 222, 128]);
});

test("spillInwardBy - left-edge shrink spills from the first member inward", () => {
    // a gives 72 (200 -> 128), b gives 172 (300 -> 128), c gives the last 106 (400 -> 294).
    assert.deepEqual(spillInwardBy([200, 300, 400], -350, MIN, true), [128, 128, 294]);
});

test("spillInwardBy - a member already below the floor has zero headroom and is skipped, never enlarged", () => {
    // Below-floor states are reachable (split/minimize-restore/window reflow
    // don't enforce the floor) — same discipline as shrinkBlockBy.
    assert.deepEqual(spillInwardBy([300, 100, 300], -200, MIN, false), [272, 100, 128]);
});

test("spillInwardBy - all members at the floor falls back to proportional scaling", () => {
    const result = spillInwardBy([128, 128, 128], -84, MIN, false);
    // (384 - 84) / 384 applied uniformly.
    result.forEach((v) => assert.approximately(v, 100, 1e-9));
    assert.approximately(sum(result), 300, 1e-9);
});

test("spillInwardBy - partial floor headroom is consumed first, then proportional fallback for the rest", () => {
    const result = spillInwardBy([200, 128, 128], -100, MIN, false);
    // Edge-first pass: c and b have no headroom, a gives 72 (200 -> 128).
    // Remaining 28 scales all three proportionally: 356/384 each.
    const factor = (384 - 28) / 384;
    assert.approximately(result[0], 128 * factor, 1e-9);
    assert.approximately(result[1], 128 * factor, 1e-9);
    assert.approximately(result[2], 128 * factor, 1e-9);
    assert.approximately(sum(result), 456 - 100, 1e-9);
});

test("spillInwardBy - never returns negative sizes even when the shrink exceeds the container", () => {
    const result = spillInwardBy([200, 200], -1000, MIN, false);
    result.forEach((v) => assert.isAtLeast(v, 0));
});

// ---------------------------------------------------------------------------
// computeWindowEdgeResizeOps — the recursive edge-chain rule
// ---------------------------------------------------------------------------

test("computeWindowEdgeResizeOps - right-edge grow on a Row routes all delta to the last child; siblings keep pixels", () => {
    const root = branch("root", FlexDirection.Row, 10, 900, 600, [
        leaf("a", 10, 300, 600),
        leaf("b", 10, 300, 600),
        leaf("c", 10, 300, 600),
    ]);
    const ops = computeWindowEdgeResizeOps(root, "right", 150, 0, MIN);
    // Every child's weight changes (the denominator changed), but resolved
    // pixels preserve a and b exactly and give c the full 150.
    const px = resolvedPx(root, ops, 1050);
    assert.approximately(px[0], 300, 1e-6, "a keeps its pixel width");
    assert.approximately(px[1], 300, 1e-6, "b keeps its pixel width");
    assert.approximately(px[2], 450, 1e-6, "c absorbs the entire delta");
    // Weight sum is preserved so sibling weights stay in the same scale.
    const m = opsMap(ops);
    assert.approximately(sum(root.children!.map((c) => m.get(c.id) ?? c.size)), 30, 1e-9);
});

test("computeWindowEdgeResizeOps - left-edge grow routes all delta to the FIRST child", () => {
    const root = branch("root", FlexDirection.Row, 10, 900, 600, [
        leaf("a", 10, 300, 600),
        leaf("b", 10, 300, 600),
        leaf("c", 10, 300, 600),
    ]);
    const ops = computeWindowEdgeResizeOps(root, "left", 150, 0, MIN);
    const px = resolvedPx(root, ops, 1050);
    assert.approximately(px[0], 450, 1e-6, "a absorbs the entire delta");
    assert.approximately(px[1], 300, 1e-6);
    assert.approximately(px[2], 300, 1e-6);
});

test("computeWindowEdgeResizeOps - nested column-in-row: width delta recurses into EVERY column child but leaves column weights alone", () => {
    const root = branch("root", FlexDirection.Row, 10, 900, 600, [
        leaf("a", 10, 300, 600),
        branch("col", FlexDirection.Column, 20, 600, 600, [
            leaf("b", 10, 600, 300),
            leaf("c", 10, 600, 300),
        ]),
    ]);
    const ops = computeWindowEdgeResizeOps(root, "right", 150, 0, MIN);
    const m = opsMap(ops);
    // Row level: a keeps 300px, col grows to 750px.
    const px = resolvedPx(root, ops, 1050);
    assert.approximately(px[0], 300, 1e-6, "a keeps its pixel width");
    assert.approximately(px[1], 750, 1e-6, "col absorbs the entire width delta");
    // Column level: b and c both span the full width — the delta reaches
    // them via geometry, NOT via weight ops; their (height) weights on the
    // column are untouched.
    assert.isFalse(m.has("b"), "column child heights are untouched by a width delta");
    assert.isFalse(m.has("c"), "column child heights are untouched by a width delta");
});

test("computeWindowEdgeResizeOps - shrink past the edge pane's floor spills inward at the Row level", () => {
    const root = branch("root", FlexDirection.Row, 10, 900, 600, [
        leaf("a", 10, 200, 600),
        leaf("b", 15, 300, 600),
        leaf("c", 20, 400, 600),
    ]);
    const ops = computeWindowEdgeResizeOps(root, "right", -350, 0, MIN);
    const px = resolvedPx(root, ops, 550);
    assert.approximately(px[0], 200, 1e-6, "a untouched — spill stopped at b");
    assert.approximately(px[1], 222, 1e-6, "b absorbs what c could not give past its floor");
    assert.approximately(px[2], 128, 1e-6, "c floored at MinNodeSizePx");
});

test("computeWindowEdgeResizeOps - all children at the floor falls back to proportional (no weight ops)", () => {
    const root = branch("root", FlexDirection.Row, 10, 384, 600, [
        leaf("a", 10, 128, 600),
        leaf("b", 10, 128, 600),
        leaf("c", 10, 128, 600),
    ]);
    const ops = computeWindowEdgeResizeOps(root, "right", -84, 0, MIN);
    // Proportional fallback keeps the pixel ratios identical, so every
    // recomputed weight equals its snapshot value — no ops at all. This is
    // exactly "converge with the plain proportional mode" from spec §3.3.
    assert.lengthOf(ops, 0);
});

test("computeWindowEdgeResizeOps - corner drag decomposes into both axis rules independently", () => {
    const root = branch("root", FlexDirection.Row, 10, 900, 600, [
        leaf("a", 10, 300, 600),
        branch("col", FlexDirection.Column, 20, 600, 600, [
            leaf("b", 10, 600, 300),
            leaf("c", 10, 600, 300),
        ]),
    ]);
    const ops = computeWindowEdgeResizeOps(root, "bottomright", 150, 100, MIN);
    // X pass: a keeps 300, col -> 750 (as in the nested test).
    const rowPx = resolvedPx(root, ops, 1050);
    assert.approximately(rowPx[0], 300, 1e-6);
    assert.approximately(rowPx[1], 750, 1e-6);
    // Y pass: the height delta passes through the Row (cross axis) into
    // every child; inside the column only the LAST child (c) grows.
    const colPx = resolvedPx(root.children![1], ops, 700);
    assert.approximately(colPx[0], 300, 1e-6, "b keeps its pixel height");
    assert.approximately(colPx[1], 400, 1e-6, "c absorbs the entire height delta");
});

test("computeWindowEdgeResizeOps - a zero/sub-pixel delta produces no operations", () => {
    const root = branch("root", FlexDirection.Row, 10, 900, 600, [
        leaf("a", 10, 450, 600),
        leaf("b", 10, 450, 600),
    ]);
    assert.lengthOf(computeWindowEdgeResizeOps(root, "right", 0, 0, MIN), 0);
    assert.lengthOf(computeWindowEdgeResizeOps(root, "right", 0.25, 0, MIN), 0);
    assert.lengthOf(computeWindowEdgeResizeOps(root, "top", 0, 0.25, MIN), 0);
});
