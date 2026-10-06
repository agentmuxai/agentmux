// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    CURSOR_GAP_PX,
    MAX_OVERSHOOT_FRACTION,
    READABLE_WIDTH_CHARS,
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
        expect(h).toEqual({ left: 900, maxWidth: 500, minWidth: 0, shift: 1, extendsPastRow: false });
    });

    it("may reach past the row by half its width in total", () => {
        expect(MAX_OVERSHOOT_FRACTION).toBe(0.5);
        const h = computePeekHorizontal({ row: ROW, viewport: VIEWPORT, naturalWidth: 900 });
        expect(h.extendsPastRow).toBe(true);
        expect(h.maxWidth).toBe(750);
    });

    // The overshoot's split follows the pane's place in the window.
    const W = { width: 1500, height: 1000 };
    const at = (left: number) => ({ left, right: left + 500, top: 300, bottom: 330 });
    const span = (row: ReturnType<typeof at>) =>
        placedSpan(computePeekHorizontal({ row, viewport: W, naturalWidth: 50_000 }), 50_000);

    it("at the window's left edge, overshoots only to the right", () => {
        expect(span(at(0))).toEqual({ left: 0, right: 750 });
    });

    it("at the window's right edge, overshoots only to the left", () => {
        expect(span(at(1000))).toEqual({ left: 750, right: 1500 });
    });

    it("centred, overshoots a quarter of the row's width on each side", () => {
        expect(span(at(500))).toEqual({ left: 375, right: 1125 });
    });

    it("in between, interpolates the split", () => {
        // A quarter of the way across: a quarter of the overshoot to the left.
        expect(span(at(250))).toEqual({ left: 250 - 62.5, right: 750 + 187.5 });
    });

    it("a panel narrower than the cap grows out of the row in the same split", () => {
        const h = computePeekHorizontal({ row: at(500), viewport: W, naturalWidth: 600 }); // centred
        expect(placedSpan(h, 600)).toEqual({ left: 450, right: 1050 });
    });

    it("gives the room a window edge takes from one side to the other", () => {
        const viewport = { width: 1000, height: 1000 };
        const row = { left: 100, right: 900, top: 300, bottom: 330 }; // centred, 800 wide: would reach -100..1100
        const s = placedSpan(computePeekHorizontal({ row, viewport, naturalWidth: 50_000 }), 50_000);
        expect(s).toEqual({ left: VIEWPORT_MARGIN_PX, right: viewport.width - VIEWPORT_MARGIN_PX });
    });

    it("never leaves the window, and always covers its row", () => {
        for (let left = 0; left <= 1000; left += 50) {
            const row = at(left);
            for (const naturalWidth of [501, 600, 700, 5000]) {
                const s = placedSpan(computePeekHorizontal({ row, viewport: W, naturalWidth }), naturalWidth);
                expect(s.left).toBeGreaterThanOrEqual(0);
                expect(s.right).toBeLessThanOrEqual(W.width);
                expect(s.left).toBeLessThanOrEqual(row.left + 1e-9);
                expect(s.right).toBeGreaterThanOrEqual(row.right - 1e-9);
            }
        }
    });

    it("stays right-aligned in the row when the window offers no more width than the row has", () => {
        const wide = { left: 0, right: 1392, top: 300, bottom: 330 };
        const h = computePeekHorizontal({ row: wide, viewport: VIEWPORT, naturalWidth: 5000 });
        expect(h.shift).toBe(1);
        expect(h.extendsPastRow).toBe(false);
    });
});

// The overshoot only reaches for a readable width (~120 characters of code):
// full for thin panes, none for panes already that wide. The decision and its
// sources: docs/reports/REPORT_TOOL_HOVER_PANEL_SIZE_AND_PLACEMENT_2026_10_02.md §9.
describe("computePeekHorizontal: readable width", () => {
    const viewport = { width: 2000, height: 1000 };
    const READABLE = 950; // ≈120ch of the panel's code font at the default size
    const widthFor = (paneWidth: number, left = 0) => {
        const row = { left, right: left + paneWidth, top: 300, bottom: 330 };
        return computePeekHorizontal({ row, viewport, naturalWidth: 50_000, readableWidth: READABLE }).maxWidth;
    };

    it("is about 120 characters", () => {
        expect(READABLE_WIDTH_CHARS).toBe(120);
    });

    it("thin panes keep the full half-width overshoot", () => {
        expect(widthFor(350)).toBe(525);
        expect(widthFor(500)).toBe(750);
    });

    it("wider panes overshoot only as far as the readable width", () => {
        expect(widthFor(700)).toBe(950);
        expect(widthFor(900)).toBe(950);
    });

    it("a pane at least the readable width gets no overshoot", () => {
        for (const paneWidth of [950, 1000, 1400]) {
            const row = { left: 300, right: 300 + paneWidth, top: 300, bottom: 330 };
            const h = computePeekHorizontal({ row, viewport, naturalWidth: 50_000, readableWidth: READABLE });
            expect(h).toMatchObject({ maxWidth: paneWidth, shift: 1, extendsPastRow: false });
        }
    });

    it("the smaller overshoot keeps the split by position", () => {
        // A 700px pane centred in a 2000px window: 250px of overshoot, half each side.
        const row = { left: 650, right: 1350, top: 300, bottom: 330 };
        const h = computePeekHorizontal({ row, viewport, naturalWidth: 50_000, readableWidth: READABLE });
        expect(placedSpan(h, 50_000)).toEqual({ left: 525, right: 1475 });
    });

    it("the 600px minimum still applies below the readable width", () => {
        const row = { left: 0, right: 500, top: 300, bottom: 330 };
        const h = computePeekHorizontal({ row, viewport, naturalWidth: 120, minWidth: 600, readableWidth: READABLE });
        expect(h.minWidth).toBe(600);
        expect(h.extendsPastRow).toBe(true);
    });

    it("without a readable width, the half-width cap alone applies (as before)", () => {
        const row = { left: 0, right: 1000, top: 300, bottom: 330 };
        expect(computePeekHorizontal({ row, viewport, naturalWidth: 50_000 }).maxWidth).toBe(1500);
    });
});

describe("computePeekHorizontal: minimum width", () => {
    it("a thin panel in a wide row stays right-aligned, at least the minimum wide", () => {
        const row = { left: 100, right: 900, top: 300, bottom: 330 }; // 800 wide
        const h = computePeekHorizontal({ row, viewport: VIEWPORT, naturalWidth: 120, minWidth: 600 });
        expect(h.shift).toBe(1);
        expect(h.minWidth).toBe(600);
        expect(placedSpan(h, 120).right - placedSpan(h, 120).left).toBe(600);
    });

    it("a thin panel in a row narrower than the minimum reaches over the pane borders", () => {
        const h = computePeekHorizontal({ row: ROW, viewport: VIEWPORT, naturalWidth: 120, minWidth: 600 }); // row 500 wide
        expect(h.minWidth).toBe(600);
        expect(h.extendsPastRow).toBe(true);
        const s = placedSpan(h, 120);
        expect(s.left).toBeLessThan(ROW.left);
        expect(s.right).toBeGreaterThan(ROW.right);
    });

    it("in a very narrow row the minimum gives way to the half-width cap", () => {
        const row = { left: 400, right: 700, top: 300, bottom: 330 }; // 300 wide: at most 450
        const h = computePeekHorizontal({ row, viewport: VIEWPORT, naturalWidth: 120, minWidth: 600 });
        expect(h.maxWidth).toBe(450);
        expect(h.minWidth).toBe(450);
    });

    it("at the window's right edge, the minimum is met by reaching left", () => {
        const row = { left: 1000, right: 1392, top: 300, bottom: 330 }; // pane at the window's right edge
        const h = computePeekHorizontal({ row, viewport: VIEWPORT, naturalWidth: 120, minWidth: 600 });
        expect(h.minWidth).toBe(588); // 392 plus half of it
        const s = placedSpan(h, 120);
        expect(s.right).toBeLessThanOrEqual(VIEWPORT.width - VIEWPORT_MARGIN_PX);
        expect(s.left).toBeCloseTo(804);
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
                            // With the minimum, as PeekOverlay places a tall panel: with the readable width too.
                            const readableWidth = minWidth ? 950 : undefined;
                            const hz = computePeekHorizontal({ row, viewport, naturalWidth, minWidth, readableWidth });
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
                            expect(s.left).toBeGreaterThanOrEqual(0);
                        }
                    });
                  }
                  }
                }
            }
        }
    }
});
