// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { assert, test } from "vitest";
import { computeGroupResizeSizes } from "../lib/layoutResize";
import type { ResizeNodeOperation } from "../lib/types";

function sizesOf(result: Map<string, number>, ids: string[]): number[] {
    return ids.map((id) => result.get(id)!);
}

function sum(values: number[]): number {
    return values.reduce((a, b) => a + b, 0);
}

/** Cumulative border position after `ids[0..index]`, in `ids`' own left-to-right order. */
function borderAfter(result: Map<string, number>, ids: string[], index: number): number {
    return sum(sizesOf(result, ids.slice(0, index + 1)));
}

test("computeGroupResizeSizes - two siblings degenerates to a plain transfer (matches the baseline 2-node math)", () => {
    const siblings: ResizeNodeOperation[] = [
        { nodeId: "a", size: 50 },
        { nodeId: "b", size: 50 },
    ];
    const result = computeGroupResizeSizes(siblings, "b", 60, 10);
    assert.equal(result.get("b"), 60);
    assert.equal(result.get("a"), 40);
    assert.equal(sum(sizesOf(result, ["a", "b"])), 100, "total size must be conserved");
});

test("computeGroupResizeSizes - driven shrinks: no border moves opposite the drag direction (regression for the reversed-border bug)", () => {
    // A(100) B(100) C(100) D(100), dragging the B|C handle so C (driven)
    // shrinks by 40. Under the old "driven vs. undifferentiated others"
    // model, D (past the driven pane) got a growth share too, which pulled
    // the C|D border LEFT — opposite the drag. The two-block model must
    // keep every border moving right (or unmoved), never left.
    const ids = ["a", "b", "c", "d"];
    const siblings: ResizeNodeOperation[] = [
        { nodeId: "a", size: 100 },
        { nodeId: "b", size: 100 },
        { nodeId: "c", size: 100 },
        { nodeId: "d", size: 100 },
    ];
    const result = computeGroupResizeSizes(siblings, "c", 60, 10);

    // beforeBlock [a, b] absorbs the full 40 (proportional 100:100 -> +20 each).
    assert.approximately(result.get("a")!, 120, 1e-6);
    assert.approximately(result.get("b")!, 120, 1e-6);
    // afterBlock [c, d] gives up 40 total, proportional 100:100 -> -20 each.
    assert.approximately(result.get("c")!, 80, 1e-6);
    assert.approximately(result.get("d")!, 80, 1e-6);
    assert.approximately(sum(sizesOf(result, ids)), 400, 1e-6, "total size must be conserved");

    const oldBorders = [100, 200, 300]; // a|b, b|c, c|d before the drag
    const newBorders = [0, 1, 2].map((i) => borderAfter(result, ids, i));
    assert.approximately(newBorders[0], 120, 1e-6, "a|b must move right, matching the drag");
    assert.approximately(newBorders[1], 240, 1e-6, "b|c (the dragged handle) must track the cursor exactly");
    assert.approximately(newBorders[2], 320, 1e-6, "c|d must move right too, not opposite the drag");
    for (let i = 0; i < oldBorders.length; i++) {
        assert.isAtLeast(newBorders[i], oldBorders[i] - 1e-6, `border ${i} must never move opposite the drag direction`);
    }
});

test("computeGroupResizeSizes - driven grows: mirror case, no border moves opposite the drag direction", () => {
    const ids = ["a", "b", "c", "d"];
    const siblings: ResizeNodeOperation[] = [
        { nodeId: "a", size: 100 },
        { nodeId: "b", size: 100 },
        { nodeId: "c", size: 100 },
        { nodeId: "d", size: 100 },
    ];
    const result = computeGroupResizeSizes(siblings, "c", 140, 10);

    assert.approximately(result.get("a")!, 80, 1e-6);
    assert.approximately(result.get("b")!, 80, 1e-6);
    assert.approximately(result.get("c")!, 120, 1e-6);
    assert.approximately(result.get("d")!, 120, 1e-6);
    assert.approximately(sum(sizesOf(result, ids)), 400, 1e-6, "total size must be conserved");

    const oldBorders = [100, 200, 300];
    const newBorders = [0, 1, 2].map((i) => borderAfter(result, ids, i));
    assert.approximately(newBorders[1], 160, 1e-6, "b|c (the dragged handle) must track the cursor exactly");
    for (let i = 0; i < oldBorders.length; i++) {
        assert.isAtMost(newBorders[i], oldBorders[i] + 1e-6, `border ${i} must never move opposite the drag direction`);
    }
});

test("computeGroupResizeSizes - beforeBlock absorbs proportional to its own sizes when afterBlock shrinks", () => {
    const siblings: ResizeNodeOperation[] = [
        { nodeId: "a", size: 10 },
        { nodeId: "b", size: 20 },
        { nodeId: "driven", size: 70 },
    ];
    // driven shrinks from 70 to 30 (afterBlock = [driven] alone, so it absorbs the full shrink);
    // beforeBlock [a, b] splits the 40 it gains proportional to their own sizes (10:20).
    const result = computeGroupResizeSizes(siblings, "driven", 30, 5);
    assert.equal(result.get("driven"), 30);
    assert.approximately(result.get("a")!, 10 + (40 * 10) / 30, 1e-6);
    assert.approximately(result.get("b")!, 20 + (40 * 20) / 30, 1e-6);
    assert.approximately(sum(sizesOf(result, ["a", "b", "driven"])), 100, 1e-6, "total size must be conserved");
});

test("computeGroupResizeSizes - afterBlock absorbs growth proportionally, so driven's own size no longer matches its raw desired value (documented trade-off)", () => {
    const siblings: ResizeNodeOperation[] = [
        { nodeId: "x", size: 30 },
        { nodeId: "driven", size: 20 },
        { nodeId: "a", size: 20 },
        { nodeId: "b", size: 30 },
    ];
    // beforeBlock = [x] shrinks by the full 10 (driven's raw desired growth) to feed
    // afterBlock = [driven, a, b], which grows by that same 10 split proportional to
    // their own sizes (20:20:30) — driven only gets its own share, not the full 10.
    const result = computeGroupResizeSizes(siblings, "driven", 30, 5);
    assert.approximately(result.get("x")!, 20, 1e-6, "beforeBlock absorbs the full pixel-implied delta — the handle tracks the cursor exactly");
    assert.approximately(result.get("driven")!, 20 + (10 * 20) / 70, 1e-6);
    assert.notEqual(result.get("driven"), 30, "driven's own size intentionally no longer matches its raw desired value once afterBlock has other members");
    assert.approximately(result.get("a")!, 20 + (10 * 20) / 70, 1e-6);
    assert.approximately(result.get("b")!, 30 + (10 * 30) / 70, 1e-6);
    assert.approximately(sum(sizesOf(result, ["x", "driven", "a", "b"])), 100, 1e-6, "total size must be conserved");
});

test("computeGroupResizeSizes - a beforeBlock member clamped to the floor drops out, and the shortfall re-spreads across the rest", () => {
    // beforeBlock = [a (near floor), b, c] must shrink to feed driven's growth.
    // a clamps almost immediately; the remainder comes from b/c only.
    const minNodeSize = 10;
    const siblings: ResizeNodeOperation[] = [
        { nodeId: "a", size: 12 }, // barely above the floor — should clamp almost immediately
        { nodeId: "b", size: 39 },
        { nodeId: "c", size: 39 },
        { nodeId: "driven", size: 10 },
    ];
    const result = computeGroupResizeSizes(siblings, "driven", 50, minNodeSize);
    assert.equal(result.get("driven"), 50, "driven should get its full requested growth — plenty of headroom exists in b/c");
    assert.isAtLeast(result.get("a")!, minNodeSize - 1e-6);
    assert.isAtLeast(result.get("b")!, minNodeSize - 1e-6);
    assert.isAtLeast(result.get("c")!, minNodeSize - 1e-6);
    assert.approximately(
        sum(sizesOf(result, ["a", "b", "c", "driven"])),
        100,
        1e-6,
        "total size must be conserved even when a sibling clamps"
    );
});

test("computeGroupResizeSizes - driven growth is capped when beforeBlock is already entirely at the floor (conservation safety)", () => {
    const minNodeSize = 10;
    const siblings: ResizeNodeOperation[] = [
        { nodeId: "a", size: 10 }, // already at the floor
        { nodeId: "b", size: 10 }, // already at the floor
        { nodeId: "driven", size: 20 },
    ];
    // Ask for a huge growth (driven -> 200) that vastly exceeds what beforeBlock can give up (0 combined).
    const result = computeGroupResizeSizes(siblings, "driven", 200, minNodeSize);
    assert.equal(result.get("driven"), 20, "no headroom exists in beforeBlock, so driven cannot actually grow");
    assert.equal(result.get("a"), minNodeSize);
    assert.equal(result.get("b"), minNodeSize);
    assert.approximately(sum(sizesOf(result, ["a", "b", "driven"])), 40, 1e-6, "total size must be conserved, not exceeded");
});

test("computeGroupResizeSizes - driven node not found in siblings is a no-op", () => {
    const siblings: ResizeNodeOperation[] = [
        { nodeId: "a", size: 50 },
        { nodeId: "b", size: 50 },
    ];
    const result = computeGroupResizeSizes(siblings, "missing", 999, 10);
    assert.equal(result.get("a"), 50);
    assert.equal(result.get("b"), 50);
});

test("computeGroupResizeSizes - zero delta is a no-op", () => {
    const siblings: ResizeNodeOperation[] = [
        { nodeId: "a", size: 33 },
        { nodeId: "b", size: 33 },
        { nodeId: "c", size: 34 },
    ];
    const result = computeGroupResizeSizes(siblings, "b", 33, 10);
    assert.equal(result.get("a"), 33);
    assert.equal(result.get("b"), 33);
    assert.equal(result.get("c"), 34);
});

test("computeGroupResizeSizes - driven with no siblings before it (degenerate: cannot happen via onResizeMove, driven is always afterNode) leaves everyone else untouched", () => {
    const siblings: ResizeNodeOperation[] = [
        { nodeId: "solo", size: 100 },
        { nodeId: "untouched", size: 50 },
    ];
    const result = computeGroupResizeSizes(siblings, "solo", 5, 10);
    assert.equal(result.get("solo"), 10, "clamped to the floor since desired (5) is below minNodeSize (10)");
    assert.equal(result.get("untouched"), 50, "no beforeBlock exists to redistribute with, so afterBlock's other members are untouched");
});
