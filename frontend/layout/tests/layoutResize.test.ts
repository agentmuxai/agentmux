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

test("computeGroupResizeSizes - growing the driven pane shrinks every other sibling proportional to its own size", () => {
    // a=10, b=20, c=70 (sum 100). Driven pane d grows from 0 (not present —
    // use a 4th sibling) to keep this simple: shrink c (the big one) via a's growth.
    const siblings: ResizeNodeOperation[] = [
        { nodeId: "a", size: 10 },
        { nodeId: "b", size: 20 },
        { nodeId: "c", size: 70 },
    ];
    // a grows by 10 (10 -> 20); b and c must give up 10 combined, proportional
    // to their own sizes (20:70, i.e. b gives up 10*20/90=2.222, c gives up 10*70/90=7.778).
    const result = computeGroupResizeSizes(siblings, "a", 20, 5);
    assert.equal(result.get("a"), 20);
    assert.approximately(result.get("b")!, 20 - (10 * 20) / 90, 1e-6);
    assert.approximately(result.get("c")!, 70 - (10 * 70) / 90, 1e-6);
    assert.approximately(sum(sizesOf(result, ["a", "b", "c"])), 100, 1e-6, "total size must be conserved");
});

test("computeGroupResizeSizes - shrinking the driven pane grows every other sibling proportionally", () => {
    const siblings: ResizeNodeOperation[] = [
        { nodeId: "a", size: 50 },
        { nodeId: "b", size: 20 },
        { nodeId: "c", size: 30 },
    ];
    // a shrinks by 20 (50 -> 30); b and c split the 20 proportional to their
    // own sizes (20:30).
    const result = computeGroupResizeSizes(siblings, "a", 30, 5);
    assert.equal(result.get("a"), 30);
    assert.approximately(result.get("b")!, 20 + (20 * 20) / 50, 1e-6);
    assert.approximately(result.get("c")!, 30 + (20 * 30) / 50, 1e-6);
    assert.approximately(sum(sizesOf(result, ["a", "b", "c"])), 100, 1e-6, "total size must be conserved");
});

test("computeGroupResizeSizes - a sibling clamped to the floor drops out, and the shortfall re-spreads across the rest", () => {
    // a is tiny (already near the floor); b and c hold the rest. Driving a
    // large growth on d should push a to its floor and take the remainder
    // from b/c only, not evenly split with a.
    const minNodeSize = 10;
    const siblings: ResizeNodeOperation[] = [
        { nodeId: "driven", size: 10 },
        { nodeId: "a", size: 12 }, // barely above the floor — should clamp almost immediately
        { nodeId: "b", size: 39 },
        { nodeId: "c", size: 39 },
    ];
    const result = computeGroupResizeSizes(siblings, "driven", 50, minNodeSize);
    assert.equal(result.get("driven"), 50, "driven should get its full requested growth — plenty of headroom exists in b/c");
    assert.isAtLeast(result.get("a")!, minNodeSize - 1e-6);
    assert.isAtLeast(result.get("b")!, minNodeSize - 1e-6);
    assert.isAtLeast(result.get("c")!, minNodeSize - 1e-6);
    assert.approximately(
        sum(sizesOf(result, ["driven", "a", "b", "c"])),
        100,
        1e-6,
        "total size must be conserved even when a sibling clamps"
    );
});

test("computeGroupResizeSizes - driven growth is capped when every other sibling is already at the floor (conservation safety)", () => {
    const minNodeSize = 10;
    const siblings: ResizeNodeOperation[] = [
        { nodeId: "driven", size: 20 },
        { nodeId: "a", size: 10 }, // already at the floor
        { nodeId: "b", size: 10 }, // already at the floor
    ];
    // Ask for a huge growth (driven -> 200) that vastly exceeds what a/b can give up (0 combined).
    const result = computeGroupResizeSizes(siblings, "driven", 200, minNodeSize);
    assert.equal(result.get("driven"), 20, "no headroom exists anywhere, so driven cannot actually grow");
    assert.equal(result.get("a"), minNodeSize);
    assert.equal(result.get("b"), minNodeSize);
    assert.approximately(sum(sizesOf(result, ["driven", "a", "b"])), 40, 1e-6, "total size must be conserved, not exceeded");
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

test("computeGroupResizeSizes - driven with no other siblings just applies the clamped desired size", () => {
    const siblings: ResizeNodeOperation[] = [{ nodeId: "solo", size: 100 }];
    const result = computeGroupResizeSizes(siblings, "solo", 5, 10);
    assert.equal(result.get("solo"), 10, "clamped to the floor since desired (5) is below minNodeSize (10)");
});
