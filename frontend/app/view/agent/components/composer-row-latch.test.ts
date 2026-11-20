// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { computeComposerRows, type ComposerRow } from "./composer-row-layout";
import { layoutFits, layoutLines, latchLayout, RESIZE_PX, type Latch, type RowsLayout } from "./composer-row-latch";

const GAP = 8;
const SLOTS = ["runtime", "auth", "ctx", "hostShell"];
const row = (left: string[], right: string[]): ComposerRow => ({ left, right });
const oneLine: RowsLayout = { rows: [row(["runtime", "auth"], ["ctx", "hostShell"])], statsInline: true };
// With no stats, `statsInline` is true exactly for a single row (computeStatsInline), so a taller layout has it false.
const twoLines: RowsLayout = { rows: [row(["runtime"], ["hostShell"]), row(["auth"], ["ctx"])], statsInline: false };

/** The measured side of a call: widths small enough that every layout here fits, unless a test says otherwise. */
const now = (over: Partial<Parameters<typeof latchLayout>[2]> = {}) => ({
    widths: { runtime: 80, auth: 80, ctx: 80, hostShell: 80 },
    statsWidth: 0,
    gapPx: GAP,
    width: 600,
    metrics: "1.000|8.0|13px",
    content: SLOTS.join(","),
    ...over,
});

describe("layoutLines", () => {
    it("counts the rows, and the stats' own line when they are not inline", () => {
        expect(layoutLines(oneLine, 0)).toBe(1);
        expect(layoutLines({ ...oneLine, statsInline: false }, 40)).toBe(2);
        expect(layoutLines({ ...oneLine, statsInline: false }, 0)).toBe(1);
        expect(layoutLines(twoLines, 0)).toBe(2);
        expect(layoutLines(twoLines, 40)).toBe(3);
    });
});

describe("latchLayout", () => {
    it("starts from the first measurement", () => {
        const latch = latchLayout(undefined, oneLine, now());
        expect(latch.layout).toBe(oneLine);
        expect(latch.width).toBe(600);
    });

    it("never lowers the count at one width: the readout flipping between one line and two stays on two", () => {
        let latch: Latch | undefined = latchLayout(undefined, oneLine, now());
        const seen: number[] = [];
        // The readout's bucket steps up and down; the fresh answer alternates.
        for (const fresh of [twoLines, oneLine, twoLines, oneLine, oneLine, twoLines, oneLine]) {
            latch = latchLayout(latch, fresh, now());
            seen.push(layoutLines(latch.layout, 0));
        }
        expect(seen).toEqual([2, 2, 2, 2, 2, 2, 2]);
    });

    it("keeps the arrangement as well as the count: nothing moves inside the same count", () => {
        const first: RowsLayout = twoLines;
        const sameCountOtherPairing: RowsLayout = { rows: [row(["runtime"], ["ctx"]), row(["auth"], ["hostShell"])], statsInline: false };
        let latch = latchLayout(undefined, first, now());
        latch = latchLayout(latch, sameCountOtherPairing, now());
        expect(latch.layout).toBe(first);
    });

    it("keeps the stats on their own line once moved there, even when they would fit again", () => {
        const evicted: RowsLayout = { rows: [row(["runtime", "auth"], ["ctx", "hostShell"])], statsInline: false };
        let latch = latchLayout(undefined, evicted, now({ statsWidth: 40 }));
        latch = latchLayout(latch, oneLine, now({ statsWidth: 40 }));
        expect(latch.layout).toBe(evicted);
    });

    it("raises the count when the content really needs more room, and keeps that count", () => {
        let latch = latchLayout(undefined, oneLine, now());
        latch = latchLayout(latch, twoLines, now());
        expect(layoutLines(latch.layout, 0)).toBe(2);
        latch = latchLayout(latch, oneLine, now());
        expect(layoutLines(latch.layout, 0)).toBe(2);
        const three: RowsLayout = { rows: [row(["runtime"], ["hostShell"]), row(["auth"], []), row([], ["ctx"])], statsInline: false };
        latch = latchLayout(latch, three, now());
        expect(layoutLines(latch.layout, 0)).toBe(3);
        latch = latchLayout(latch, twoLines, now());
        expect(layoutLines(latch.layout, 0)).toBe(3);
    });

    it("releases when the width changes by a resize, and starts again from the fresh layout", () => {
        let latch = latchLayout(undefined, twoLines, now({ width: 600 }));
        latch = latchLayout(latch, oneLine, now({ width: 600 + RESIZE_PX }));
        expect(latch.layout).toBe(oneLine);
        expect(latch.width).toBe(600 + RESIZE_PX);
        // And from there it latches to the tallest seen since the release.
        latch = latchLayout(latch, twoLines, now({ width: 600 + RESIZE_PX }));
        latch = latchLayout(latch, oneLine, now({ width: 600 + RESIZE_PX }));
        expect(layoutLines(latch.layout, 0)).toBe(2);
    });

    it("does not release for a sub-pixel wobble, measured from where it was latched", () => {
        let latch = latchLayout(undefined, twoLines, now({ width: 600 }));
        for (const w of [600.4, 599.6, 600.9, 599.2, 600]) latch = latchLayout(latch, oneLine, now({ width: w }));
        expect(latch.layout).toBe(twoLines);
        expect(latch.width).toBe(600);
    });

    it("takes the fresh layout while the strip is settling after a release, even a shorter one at the same width", () => {
        let latch = latchLayout(undefined, twoLines, now({ width: 600 }));
        latch = latchLayout(latch, twoLines, now({ width: 601, settling: true }));
        // The measuring that the new width set off finishes, and the layout it ends on is one line.
        latch = latchLayout(latch, oneLine, now({ width: 601, settling: true }));
        expect(latch.layout).toBe(oneLine);
        // Settled: from here the count does not go down again.
        latch = latchLayout(latch, twoLines, now({ width: 601 }));
        latch = latchLayout(latch, oneLine, now({ width: 601 }));
        expect(layoutLines(latch.layout, 0)).toBe(2);
    });

    it("releases when the measuring conditions change (a zoom or a font size changes every width)", () => {
        let latch = latchLayout(undefined, twoLines, now());
        latch = latchLayout(latch, oneLine, now({ metrics: "1.250|10.0|16px" }));
        expect(latch.layout).toBe(oneLine);
    });

    it("releases when the set of slots changes: a different strip, not a wobble", () => {
        let latch = latchLayout(undefined, twoLines, now());
        latch = latchLayout(latch, oneLine, now({ content: "runtime,ctx,hostShell" }));
        expect(latch.layout).toBe(oneLine);
    });

    it("uses the fresh layout when a slot has grown past what the latched one holds", () => {
        const latched = latchLayout(undefined, oneLine, now());
        // 4 slots of 200 px and three gaps do not fit 600 px on one row any more.
        const wide = now({ widths: { runtime: 200, auth: 200, ctx: 200, hostShell: 200 } });
        expect(layoutFits(oneLine, wide)).toBe(false);
        expect(latchLayout(latched, twoLines, wide).layout).toBe(twoLines);
    });

    it("lets a hidden pane come back to the same layout: no time in it, nothing to release", () => {
        const latched = latchLayout(undefined, twoLines, now({ width: 600 }));
        // Hidden: the strip reports no width and the caller does not call; shown again at the same width:
        expect(latchLayout(latched, oneLine, now({ width: 600 })).layout).toBe(twoLines);
    });

    it("is per strip: one latch does not touch another", () => {
        const a = latchLayout(undefined, twoLines, now());
        const b = latchLayout(undefined, oneLine, now());
        expect(a.layout).toBe(twoLines);
        expect(b.layout).toBe(oneLine);
    });
});

describe("layoutFits", () => {
    it("is false when a row is wider than the strip, and when inline stats no longer fit beside the single row", () => {
        expect(layoutFits(oneLine, now())).toBe(true);
        expect(layoutFits(oneLine, now({ width: 300 }))).toBe(false);
        // Four slots of 80 and three gaps are 344; 344 + 8 + 300 does not fit 600.
        expect(layoutFits(oneLine, now({ statsWidth: 300 }))).toBe(false);
        expect(layoutFits({ ...oneLine, statsInline: false }, now({ statsWidth: 300 }))).toBe(true);
    });
});

describe("with the real row pairing", () => {
    // The readout beside the context reading is one unit whose width steps by a bucket as its text changes.
    const rowsFor = (unitWidth: number, available: number): RowsLayout => {
        const rows = computeComposerRows(
            [
                { key: "runtime", width: 120 },
                { key: "auth", width: 96 },
                { key: "ctx", width: unitWidth },
                { key: "hostShell", width: 88 },
            ],
            "hostShell",
            available,
            GAP,
            "runtime",
            [{ leftKey: "auth", rightKey: "ctx" }],
        );
        return { rows, statsInline: rows.length === 1 };
    };

    it("the fresh layout flips with the readout; the latched one does not", () => {
        const available = 520;
        // 120 + 96 + unit + 88 + 3 gaps is 328 + unit: one row up to 192, more above it.
        const sizes = [184, 192, 200, 192, 200, 184, 192];
        const fresh = sizes.map((w) => layoutLines(rowsFor(w, available), 0));
        expect(new Set(fresh).size).toBeGreaterThan(1);
        let latch: Latch | undefined;
        const shown: number[] = [];
        for (const w of sizes) {
            const f = rowsFor(w, available);
            latch = latchLayout(latch, f, {
                widths: { runtime: 120, auth: 96, ctx: w, hostShell: 88 },
                statsWidth: 0,
                gapPx: GAP,
                width: available,
                metrics: "m",
                content: SLOTS.join(","),
            });
            shown.push(layoutLines(latch.layout, 0));
        }
        // Never lower than the highest the readout has made it at this width.
        let highest = 0;
        sizes.forEach((_, i) => {
            highest = Math.max(highest, fresh[i]);
            expect(shown[i]).toBeGreaterThanOrEqual(highest);
        });
        expect(new Set(shown.slice(fresh.indexOf(Math.max(...fresh)))).size).toBe(1);
    });
});
