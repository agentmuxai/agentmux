// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    CURSOR_GAP_PX,
    VIEWPORT_MARGIN_PX,
    computePeekHorizontal,
    computePeekVertical,
    placedExtent,
    placedSpan,
} from "./peek-placement";

const ROW = { left: 400, right: 900, top: 300, bottom: 330 };
const VIEWPORT = { width: 1400, height: 1000 };
const CONTAINER = { top: 100, bottom: 900 };

describe("computePeekHorizontal", () => {
    it("right-aligns to the row when the panel fits within the row's width", () => {
        const h = computePeekHorizontal({ row: ROW, viewport: VIEWPORT, naturalWidth: 300 });
        expect(h).toEqual({ left: 900, maxWidth: 500, minWidth: 0, alignRight: true, extendsPastRow: false });
    });

    it("pins to the row's LEFT edge and extends right over the pane border when wider than the row", () => {
        const h = computePeekHorizontal({ row: ROW, viewport: VIEWPORT, naturalWidth: 900 });
        expect(h.alignRight).toBe(false);
        expect(h.left).toBe(400);
        expect(h.extendsPastRow).toBe(true);
        // Uses the room to the window's right edge: 1400 - 8 - 400.
        expect(h.maxWidth).toBe(VIEWPORT.width - VIEWPORT_MARGIN_PX - 400);
    });

    it("uses as much width as exists, however long the line", () => {
        const h = computePeekHorizontal({ row: ROW, viewport: VIEWPORT, naturalWidth: 50_000 });
        expect(placedSpan(h, 50_000).right).toBe(VIEWPORT.width - VIEWPORT_MARGIN_PX);
    });

    it("never grows past the window's right edge", () => {
        const h = computePeekHorizontal({ row: ROW, viewport: VIEWPORT, naturalWidth: 700 });
        expect(placedSpan(h, 700).right).toBeLessThanOrEqual(VIEWPORT.width - VIEWPORT_MARGIN_PX);
    });

    it("stays right-aligned in the row when the window offers no more width than the row has", () => {
        const wide = { left: 0, right: 1392, top: 300, bottom: 330 };
        const h = computePeekHorizontal({ row: wide, viewport: VIEWPORT, naturalWidth: 5000 });
        expect(h.alignRight).toBe(true);
        expect(h.extendsPastRow).toBe(false);
    });
});

describe("computePeekHorizontal: minimum width", () => {
    it("a thin panel in a wide row stays right-aligned, at least the minimum wide", () => {
        const row = { left: 100, right: 900, top: 300, bottom: 330 }; // 800 wide
        const h = computePeekHorizontal({ row, viewport: VIEWPORT, naturalWidth: 120, minWidth: 600 });
        expect(h.alignRight).toBe(true);
        expect(h.minWidth).toBe(600);
        expect(placedSpan(h, 120).right - placedSpan(h, 120).left).toBe(600);
    });

    it("a thin panel in a row narrower than the minimum goes left-pinned, over the pane border", () => {
        const h = computePeekHorizontal({ row: ROW, viewport: VIEWPORT, naturalWidth: 120, minWidth: 600 }); // row 500 wide
        expect(h.alignRight).toBe(false);
        expect(h.left).toBe(400);
        expect(h.minWidth).toBe(600);
        expect(h.extendsPastRow).toBe(true);
    });

    it("never runs off the window: the minimum is capped to the room there is", () => {
        const row = { left: 1000, right: 1392, top: 300, bottom: 330 }; // pane at the window's right edge
        const h = computePeekHorizontal({ row, viewport: VIEWPORT, naturalWidth: 120, minWidth: 600 });
        expect(h.minWidth).toBe(392);
        expect(placedSpan(h, 120).right).toBeLessThanOrEqual(VIEWPORT.width);
        expect(placedSpan(h, 120).left).toBeGreaterThanOrEqual(0);
    });
});

describe("computePeekVertical", () => {
    const base = { row: ROW, container: CONTAINER, viewport: VIEWPORT };

    it("places a short panel below the pointer, inside the container", () => {
        const v = computePeekVertical({ ...base, mouseY: 315, naturalHeight: 60 });
        expect(v.top).toBe(315 + CURSOR_GAP_PX);
        expect(v.leavesContainer).toBe(false);
        expect(v.scrolls).toBe(false);
    });

    it("flips above the pointer when it only fits there", () => {
        const row = { left: 400, right: 900, top: 840, bottom: 870 };
        const v = computePeekVertical({ ...base, row, mouseY: 850, naturalHeight: 200 });
        expect(v.top + 200).toBe(850 - CURSOR_GAP_PX);
        expect(v.leavesContainer).toBe(false);
    });

    it("stays near the pointer when too tall for the container: leaves the pane, never covers the pointer", () => {
        const v = computePeekVertical({ ...base, mouseY: 500, naturalHeight: 900 });
        const e = placedExtent(v);
        expect(e.top > 500 || e.bottom < 500).toBe(true);
        expect(v.leavesContainer).toBe(true);
        // Below the pointer (more room), reaching toward the window's bottom edge.
        expect(e.top).toBe(500 + CURSOR_GAP_PX);
        expect(e.bottom).toBeLessThanOrEqual(VIEWPORT.height - VIEWPORT_MARGIN_PX);
    });

    it("uses ALL the room on the chosen side, and scrolls the rest", () => {
        const v = computePeekVertical({ ...base, mouseY: 500, naturalHeight: 5000 });
        expect(v.scrolls).toBe(true);
        expect(v.maxHeight).toBe(VIEWPORT.height - VIEWPORT_MARGIN_PX - (500 + CURSOR_GAP_PX));
    });

    it("goes above when above has more room", () => {
        const row = { left: 400, right: 900, top: 840, bottom: 870 };
        const v = computePeekVertical({
            ...base,
            row,
            container: { top: 600, bottom: 900 },
            mouseY: 850,
            naturalHeight: 5000,
        });
        expect(v.top + v.maxHeight).toBe(850 - CURSOR_GAP_PX);
        expect(v.scrolls).toBe(true);
    });

    it("prefers below when the panel fits there, even past the container", () => {
        const v = computePeekVertical({
            ...base,
            container: { top: 100, bottom: 400 },
            mouseY: 350,
            naturalHeight: 500,
        });
        expect(v.top).toBe(350 + CURSOR_GAP_PX);
        expect(v.scrolls).toBe(false);
        expect(v.leavesContainer).toBe(true);
    });

    it("sits at the row's top when there is no pointer position yet", () => {
        const v = computePeekVertical({ ...base, mouseY: null, naturalHeight: 100 });
        expect(v.top).toBe(300);
    });

    // flushToRow: a panel the pointer must be able to ENTER keeps its near edge on
    // the row, so there is no strip of the next row between the two.
    describe("flushToRow", () => {
        it("below the pointer: starts at the row's bottom when pointer + gap would pass it", () => {
            const v = computePeekVertical({ ...base, mouseY: 325, naturalHeight: 5000, flushToRow: true });
            expect(v.top).toBe(330); // ROW.bottom, not 325 + 12 = 337
            expect(v.scrolls).toBe(true);
        });

        it("below the pointer: keeps pointer + gap when that is still on the row", () => {
            const v = computePeekVertical({ ...base, mouseY: 305, naturalHeight: 5000, flushToRow: true });
            expect(v.top).toBe(305 + CURSOR_GAP_PX);
        });

        it("above the pointer: ends at the row's top when pointer - gap would pass it", () => {
            const row = { left: 400, right: 900, top: 840, bottom: 870 };
            const v = computePeekVertical({
                ...base, row, container: { top: 600, bottom: 900 }, mouseY: 845, naturalHeight: 5000, flushToRow: true,
            });
            expect(v.top + v.maxHeight).toBe(840); // ROW.top, not 845 - 12 = 833
        });

        it("off by default: pointer + gap, as before", () => {
            const v = computePeekVertical({ ...base, mouseY: 325, naturalHeight: 5000 });
            expect(v.top).toBe(325 + CURSOR_GAP_PX);
        });
    });
});

// The invariant this file exists for. The panel is portalled, so the row sees
// `mouseleave` when the pointer enters it; a panel over the pointer flickers.
// Sweep pointer position x panel size x window size x pane position and assert
// the pointer is never inside the placed panel.
describe("the pointer is never inside the placed panel", () => {
    const viewports = [
        { width: 1400, height: 1000 },
        { width: 1000, height: 600 },
        { width: 700, height: 500 },
        { width: 2000, height: 1200 },
    ];
    const heights = [20, 100, 250, 400, 700, 1000, 5000];
    const widths = [200, 600, 3000];

    for (const viewport of viewports) {
        const panes = [
            { left: 0, right: viewport.width },
            { left: 0, right: Math.floor(viewport.width / 2) },
            { left: Math.floor(viewport.width / 2), right: viewport.width },
            { left: Math.floor(viewport.width * 0.35), right: Math.floor(viewport.width * 0.65) },
        ];
        for (const pane of panes) {
            for (const naturalHeight of heights) {
                for (const naturalWidth of widths) {
                  for (const flushToRow of [false, true]) {
                  for (const minWidth of [0, 600]) {
                    it(`${viewport.width}x${viewport.height} pane ${pane.left}-${pane.right}, panel ${naturalWidth}x${naturalHeight}${flushToRow ? ", flush" : ""}${minWidth ? ", min 600" : ""}`, () => {
                        const container = { top: 60, bottom: viewport.height - 80 };
                        for (let y = container.top + 1; y < container.bottom; y += 7) {
                            const row = { ...pane, top: y - 10, bottom: y + 10 };
                            const hz = computePeekHorizontal({ row, viewport, naturalWidth, minWidth });
                            const v = computePeekVertical({ row, mouseY: y, container, viewport, naturalHeight, flushToRow });
                            const e = placedExtent(v);
                            const s = placedSpan(hz, naturalWidth);
                            for (const x of [pane.left + 1, (pane.left + pane.right) / 2, pane.right - 1]) {
                                const inside = y >= e.top && y <= e.bottom && x >= s.left && x <= s.right;
                                expect(
                                    inside,
                                    `pointer=(${x},${y}) panel x[${s.left},${s.right}] y[${e.top},${e.bottom}]`,
                                ).toBe(false);
                            }
                            // And it never leaves the window.
                            expect(e.bottom).toBeLessThanOrEqual(viewport.height);
                            expect(e.top).toBeGreaterThanOrEqual(0);
                            expect(s.right).toBeLessThanOrEqual(viewport.width);
                        }
                    });
                  }
                  }
                }
            }
        }
    }
});
