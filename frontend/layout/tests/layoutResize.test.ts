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

test("computeGroupResizeSizes - a block member already below minNodeSize is left untouched, never enlarged, when its block shrinks (regression: Codex review on PR #2624)", () => {
    // Split/minimize-restore/window-shrink reflow don't enforce minNodeSize — only
    // this interactive drag path does — so a pane can genuinely already be smaller
    // than the floor when a Shift-drag starts. Shrinking a block containing one
    // must not "fix" it by enlarging it up to the floor (that's growth, the
    // opposite of what this helper does, and it broke total-size conservation).
    // tiny and other share beforeBlock; driven grows, so beforeBlock is the one
    // asked to shrink (this is where tiny's stale undersized state matters).
    const minNodeSize = 128;
    const siblings: ResizeNodeOperation[] = [
        { nodeId: "tiny", size: 20 }, // already below the floor before the drag starts
        { nodeId: "other", size: 200 },
        { nodeId: "driven", size: 200 },
    ];
    // Modest growth request (driven -> 250, beforeBlock must give up 50) — other alone
    // has plenty of headroom (72) to cover it without ever touching tiny.
    const modest = computeGroupResizeSizes(siblings, "driven", 250, minNodeSize);
    assert.equal(modest.get("tiny"), 20, "already-undersized sibling must be left exactly as-is, not enlarged");
    assert.approximately(modest.get("other")!, 150, 1e-6);
    assert.approximately(modest.get("driven")!, 250, 1e-6, "driven gets its full requested growth — plenty of real headroom exists in other");
    assert.approximately(sum(sizesOf(modest, ["tiny", "other", "driven"])), 420, 1e-6, "total size must be conserved");

    // Larger growth request that exceeds other's real headroom (72, down to its own
    // floor) — must cap at that real headroom, not "find" extra room by enlarging tiny,
    // and must never let the block's total size grow while it's being asked to shrink.
    const large = computeGroupResizeSizes(siblings, "driven", 300, minNodeSize);
    assert.equal(large.get("tiny"), 20, "still untouched even when the request would otherwise exceed available headroom");
    assert.approximately(large.get("other")!, 128, 1e-6, "other capped at its own real floor headroom");
    assert.approximately(large.get("driven")!, 272, 1e-6, "driven only grows by the real 72 of headroom that existed, not the full 100 requested");
    assert.approximately(
        sum(sizesOf(large, ["tiny", "other", "driven"])),
        420,
        1e-6,
        "total size must be conserved — the block must never net-grow while being asked to shrink"
    );
});

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

test("computeGroupResizeSizes - driven reaches the true floor even when the raw cursor position implies it should go far below minNodeSize (regression: totalDelta must not be pre-clamped on driven's own raw desired size)", () => {
    // afterBlock = [driven, far], both starting at 200 with a floor of 128 (headroom 72 each,
    // 144 combined). beforeBlock = [x] has effectively unlimited headroom (only grows here).
    // Dragging far enough that the RAW cursor-implied desired size for driven alone (50) is
    // already below the floor must still let driven and far share the block's full 144 of
    // real headroom — landing driven exactly at 128, not stalled wherever it happened to be
    // when the raw desired first crossed the floor (the bug: pre-clamping drivenDesiredSize
    // to minNodeSize before computing totalDelta freezes the aggregate delta the instant the
    // UNSHARED raw position crosses the floor, even though driven's REAL size — shared
    // proportionally with `far` — hasn't gotten there yet).
    const minNodeSize = 128;
    const siblings: ResizeNodeOperation[] = [
        { nodeId: "x", size: 1000 },
        { nodeId: "driven", size: 200 },
        { nodeId: "far", size: 200 },
    ];
    const result = computeGroupResizeSizes(siblings, "driven", 50, minNodeSize);
    assert.approximately(result.get("driven")!, 128, 1e-6, "driven must reach the actual floor, not stall above it");
    assert.approximately(result.get("far")!, 128, 1e-6, "far shares the same block and floor headroom as driven");
    assert.approximately(result.get("x")!, 1144, 1e-6, "beforeBlock absorbs the full 144 of real headroom the block could give up");
    assert.approximately(
        sum(sizesOf(result, ["x", "driven", "far"])),
        1400,
        1e-6,
        "total size must be conserved"
    );

    // Dragging even further past the floor (raw desired well below 50) must not change the
    // outcome — the block is already fully floored, this just re-confirms the cap holds.
    const resultFurther = computeGroupResizeSizes(siblings, "driven", -1000, minNodeSize);
    assert.approximately(resultFurther.get("driven")!, 128, 1e-6);
    assert.approximately(resultFurther.get("far")!, 128, 1e-6);
    assert.approximately(resultFurther.get("x")!, 1144, 1e-6);
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
