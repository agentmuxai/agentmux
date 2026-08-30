// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    computeDragPreviewSize,
    dragPreviewCursorOffset,
    DRAG_PREVIEW_FALLBACK,
    DRAG_PREVIEW_MAX_PX,
    DRAG_PREVIEW_MIN_PX,
} from "./drag-preview-size";

const ratio = (s: { width: number; height: number }) => s.width / s.height;

describe("computeDragPreviewSize", () => {
    it("preserves the pane's aspect ratio — the point of the change", () => {
        // A wide terminal must read as wide, not as the old 300x300 square.
        const s = computeDragPreviewSize({ width: 1200, height: 400 });
        expect(ratio(s)).toBeCloseTo(3, 2);
        expect(Math.max(s.width, s.height)).toBe(DRAG_PREVIEW_MAX_PX);
    });

    it("preserves a tall pane's ratio too", () => {
        const s = computeDragPreviewSize({ width: 400, height: 1200 });
        expect(ratio(s)).toBeCloseTo(1 / 3, 2);
        expect(Math.max(s.width, s.height)).toBe(DRAG_PREVIEW_MAX_PX);
    });

    it("caps the longest edge so the ghost can't occlude the drop targets", () => {
        // The reason literal pane size was rejected: a half-screen ghost sits
        // over the split indicators and tab strip you are aiming at.
        const s = computeDragPreviewSize({ width: 3840, height: 2160 });
        expect(s.width).toBe(DRAG_PREVIEW_MAX_PX);
        expect(s.height).toBeLessThan(DRAG_PREVIEW_MAX_PX);
    });

    it("never scales a small pane up", () => {
        const s = computeDragPreviewSize({ width: 200, height: 150 });
        expect(s).toEqual({ width: 200, height: 150 });
    });

    it("floors an extreme ratio's short edge rather than rendering a sliver", () => {
        // 4000x128 (128 = layoutResize.ts's MinNodeSizePx, so the narrowest
        // pane that can really exist) would scale to 360x11.5 — a line, not a
        // pane. Breaking the ratio here is deliberate.
        const s = computeDragPreviewSize({ width: 4000, height: 128 });
        expect(s.height).toBe(DRAG_PREVIEW_MIN_PX);
        expect(s.width).toBe(DRAG_PREVIEW_MAX_PX);
    });

    it("never inflates a pane that is small on BOTH axes", () => {
        // The floor used to be applied unconditionally to each axis, so this
        // came back 96x96 — larger than the pane being dragged. The function
        // does not know about layoutResize.ts's 128px minimum and must not
        // depend on it.
        expect(computeDragPreviewSize({ width: 50, height: 30 })).toEqual({ width: 50, height: 30 });
    });

    it("never lets the floor exceed the pane's own short edge", () => {
        // A downscale DID happen here, so the floor is live — but 50 < 96, and
        // returning 96 would enlarge an axis of a pane that is genuinely 50px.
        const s = computeDragPreviewSize({ width: 4000, height: 50 });
        expect(s).toEqual({ width: DRAG_PREVIEW_MAX_PX, height: 50 });
    });

    it("has no discontinuity at the max-edge boundary", () => {
        // One px of pane width used to flip the ghost from 360x40 to 360x96.
        expect(computeDragPreviewSize({ width: 360, height: 40 })).toEqual(
            computeDragPreviewSize({ width: 361, height: 40 })
        );
    });

    it("falls back to the previous fixed square when the rect is unusable", () => {
        for (const bad of [null, undefined, { width: 0, height: 0 }, { width: NaN, height: 10 }]) {
            expect(computeDragPreviewSize(bad as never)).toEqual(DRAG_PREVIEW_FALLBACK);
        }
    });

    it("returns integral px — a fractional drag image rasterises blurry", () => {
        const s = computeDragPreviewSize({ width: 1000.4, height: 333.3 });
        expect(Number.isInteger(s.width)).toBe(true);
        expect(Number.isInteger(s.height)).toBe(true);
    });
});

describe("dragPreviewCursorOffset", () => {
    it("matches the original formula so the grab point is unchanged at 300x300", () => {
        // Pins the pre-existing behaviour: (300*2 - 300)/2 + 10 = 160.
        const o = dragPreviewCursorOffset({ width: 300, height: 300 }, 2);
        expect(o).toEqual({ x: 160, y: 160 });
    });

    it("scales with the actual image size, not a nominal constant", () => {
        // The failure this prevents: offsets computed from a stale constant
        // while the image is a different size detaches the ghost from cursor.
        const wide = dragPreviewCursorOffset({ width: 360, height: 120 }, 2);
        expect(wide.x).toBe(190);
        expect(wide.y).toBe(70);
    });

    it("treats a missing or nonsense dpr as 1", () => {
        for (const bad of [0, -1, NaN, undefined as unknown as number]) {
            expect(dragPreviewCursorOffset({ width: 300, height: 200 }, bad)).toEqual({ x: 10, y: 10 });
        }
    });
});
