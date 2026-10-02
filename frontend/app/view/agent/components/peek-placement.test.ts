// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    CURSOR_GAP_PX,
    MIN_SIDE_WIDTH_PX,
    PEEK_MAX_HEIGHT_VH,
    computePeekPlacement,
    placedExtent,
    placedSpan,
    type PeekPlacementInput,
} from "./peek-placement";

const PANEL_WIDTH = 500;

function input(over: Partial<PeekPlacementInput>): PeekPlacementInput {
    return {
        row: { left: 400, right: 900, top: 300, bottom: 330 },
        mouseY: 315,
        container: { top: 100, bottom: 900 },
        viewport: { width: 1400, height: 1000 },
        naturalHeight: 100,
        currentHeight: 100,
        ...over,
    };
}

describe("computePeekPlacement", () => {
    it("keeps a short panel inside the pane, below the pointer", () => {
        const p = computePeekPlacement(input({ naturalHeight: 60 }));
        expect(p.mode).toBe("inside");
        expect(p.alignRight).toBe(true);
        expect(p.left).toBe(900);
        expect(p.top).toBe(315 + CURSOR_GAP_PX);
    });

    it("flips above the pointer when it only fits there", () => {
        const p = computePeekPlacement(
            input({ row: { left: 400, right: 900, top: 840, bottom: 870 }, mouseY: 850, naturalHeight: 200 }),
        );
        expect(p.mode).toBe("inside");
        expect(p.top + 200).toBe(850 - CURSOR_GAP_PX);
    });

    it("goes beside the pane, not over the pointer, when it fits neither side", () => {
        const p = computePeekPlacement(input({ mouseY: 500, naturalHeight: 900, currentHeight: 600 }));
        expect(p.mode).toBe("beside");
        // Right of the pane (row.right = 900): 500px of room, more than the left's 394.
        expect(p.left).toBeGreaterThan(900);
        expect(p.alignRight).toBe(false);
        expect(p.maxHeight).toBeLessThanOrEqual(1000 * PEEK_MAX_HEIGHT_VH);
    });

    it("uses the left side when the pane is at the window's right edge", () => {
        const p = computePeekPlacement(
            input({ row: { left: 700, right: 1400, top: 300, bottom: 330 }, mouseY: 500, naturalHeight: 900 }),
        );
        expect(p.mode).toBe("beside");
        expect(p.alignRight).toBe(true);
        expect(p.left).toBeLessThan(700);
    });

    it("constrains the height instead of overlapping the pointer when the pane fills the window", () => {
        const p = computePeekPlacement(
            input({
                row: { left: 0, right: 1400, top: 300, bottom: 330 },
                viewport: { width: 1400, height: 1000 },
                mouseY: 500,
                naturalHeight: 900,
            }),
        );
        expect(p.mode).toBe("vertical");
        const e = placedExtent(p, 900);
        expect(e.top > 500 || e.bottom < 500).toBe(true);
        expect(p.maxHeight).toBeLessThan(900);
    });

    it("caps the height at a share of the window, so a huge command scrolls", () => {
        const p = computePeekPlacement(input({ mouseY: 500, naturalHeight: 50_000, currentHeight: 50_000 }));
        expect(p.maxHeight).toBeLessThanOrEqual(1000 * PEEK_MAX_HEIGHT_VH);
    });

    it("keeps a beside panel on screen when the row is near the bottom", () => {
        const p = computePeekPlacement(
            input({
                row: { left: 400, right: 900, top: 950, bottom: 980 },
                // A short transcript: 600px of panel fits neither above (960-12-600 < 600) nor below.
                container: { top: 600, bottom: 1000 },
                mouseY: 960,
                naturalHeight: 900,
                currentHeight: 600,
            }),
        );
        expect(p.mode).toBe("beside");
        expect(p.top + Math.min(600, p.maxHeight)).toBeLessThanOrEqual(1000);
        expect(p.top).toBeGreaterThanOrEqual(0);
    });

    it("never goes beside when the side is narrower than the minimum", () => {
        const p = computePeekPlacement(
            input({
                row: { left: 8 + 6 + MIN_SIDE_WIDTH_PX - 1, right: 1400 - 8 - 6 - MIN_SIDE_WIDTH_PX + 1, top: 300, bottom: 330 },
                mouseY: 500,
                naturalHeight: 900,
            }),
        );
        expect(p.mode).toBe("vertical");
    });

    it("sits at the row's top when there is no pointer position yet", () => {
        const p = computePeekPlacement(input({ mouseY: null }));
        expect(p.mode).toBe("inside");
        expect(p.top).toBe(300);
    });
});

// The invariant this file exists for. The panel is portalled, so the row sees
// `mouseleave` when the pointer enters it; a panel over the pointer flickers.
// Sweep pointer position x panel height x window size x pane position and
// assert the pointer is never inside the placed panel.
describe("computePeekPlacement — the pointer is never inside the panel", () => {
    const viewports = [
        { width: 1400, height: 1000 },
        { width: 1000, height: 600 },
        { width: 700, height: 500 },
        { width: 2000, height: 1200 },
    ];
    const heights = [20, 100, 250, 400, 700, 1000, 5000];

    for (const viewport of viewports) {
        // Pane placements: fills the window, left half, right half, narrow centre.
        const panes = [
            { left: 0, right: viewport.width },
            { left: 0, right: Math.floor(viewport.width / 2) },
            { left: Math.floor(viewport.width / 2), right: viewport.width },
            { left: Math.floor(viewport.width * 0.35), right: Math.floor(viewport.width * 0.65) },
        ];
        for (const pane of panes) {
            for (const naturalHeight of heights) {
                it(`${viewport.width}x${viewport.height} pane ${pane.left}-${pane.right}, panel ${naturalHeight}px`, () => {
                    const container = { top: 60, bottom: viewport.height - 80 };
                    for (let y = container.top + 1; y < container.bottom; y += 7) {
                        const row = { ...pane, top: y - 10, bottom: y + 10 };
                        for (const x of [pane.left + 1, (pane.left + pane.right) / 2, pane.right - 1]) {
                            const p = computePeekPlacement({
                                row,
                                mouseY: y,
                                container,
                                viewport,
                                naturalHeight,
                                currentHeight: Math.min(naturalHeight, 600),
                            });
                            const e = placedExtent(p, p.mode === "beside" ? Math.min(naturalHeight, 600) : naturalHeight);
                            const s = placedSpan(p, PANEL_WIDTH);
                            const insideY = y >= e.top && y <= e.bottom;
                            const insideX = x >= s.left && x <= s.right;
                            expect(
                                insideY && insideX,
                                `mode=${p.mode} pointer=(${x},${y}) panel x[${s.left},${s.right}] y[${e.top},${e.bottom}]`,
                            ).toBe(false);
                        }
                    }
                });
            }
        }
    }
});
