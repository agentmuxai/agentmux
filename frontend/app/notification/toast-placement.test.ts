// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { placeToasts, type Box, type PlacementInput } from "./toast-placement";

// A 1000 x 800 window; the stack is 380 x 200 at right 58, bottom 34: it covers x 562..942, y 566..766.
const base: PlacementInput = {
    viewportWidth: 1000,
    viewportHeight: 800,
    width: 380,
    height: 200,
    right: 58,
    bottom: 34,
    top: 40,
    avoid: [],
};
const box = (left: number, top: number, right: number, bottom: number): Box => ({ left, top, right, bottom });

describe("placeToasts", () => {
    it("stays bottom-right with nothing to keep clear", () => {
        expect(placeToasts(base)).toEqual({ bottom: 34 });
    });

    it("stays put for a block that is somewhere else", () => {
        // Left of the stack; above it; below the window; an empty one.
        const avoid = [box(0, 600, 500, 780), box(600, 100, 900, 500), box(600, 900, 900, 1000), box(700, 600, 700, 700)];
        expect(placeToasts({ ...base, avoid })).toEqual({ bottom: 34 });
    });

    it("moves above a drawer's action block at the bottom right, with a gap", () => {
        // The drawer's action block: x 600..1000, y 650..790.
        const out = placeToasts({ ...base, avoid: [box(600, 650, 1000, 790)] });
        // Its top is 650: the stack's bottom edge is 8 above, so 800 - (650 - 8) = 158 from the window's bottom.
        expect(out).toEqual({ bottom: 158 });
    });

    it("moves above the agent composer, which spans the pane's width", () => {
        const out = placeToasts({ ...base, avoid: [box(0, 700, 1000, 790)] });
        expect(out).toEqual({ bottom: 108 });
    });

    it("clears two blocks stacked in the way", () => {
        // Raising above the lower block brings the upper one into the way.
        const out = placeToasts({ ...base, avoid: [box(600, 650, 1000, 790), box(600, 400, 1000, 560)] });
        // Above the higher block (top 400): bottom edge at 392, so 800 - 392 = 408 from the bottom.
        expect(out).toEqual({ bottom: 408 });
    });

    it("keeps the usual place when there is no room above the block", () => {
        // A block from y 100 down: above it there is no room for 200 px (100 - 8 - 200 < 40).
        const out = placeToasts({ ...base, avoid: [box(600, 100, 1000, 790)] });
        expect(out).toEqual({ bottom: 34 });
    });

    it("returns to the usual place once the block is gone", () => {
        expect(placeToasts({ ...base, avoid: [box(600, 650, 1000, 790)] })).toEqual({ bottom: 158 });
        expect(placeToasts({ ...base, avoid: [] })).toEqual({ bottom: 34 });
    });
});
