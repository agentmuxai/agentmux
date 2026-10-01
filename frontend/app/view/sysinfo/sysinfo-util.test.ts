// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import {
    buildPlotAxisLabelOptions,
    computeAutoMaxY,
    computeLeftMarginForTicks,
    computePlotMargins,
    xTickFitsBeforeRightEdge,
    PLOT_EDGE_PAD_PX,
    PLOT_TICK_GUTTER_PX,
    getGapThresholdMs,
    resolveDomainBound,
} from "./sysinfo-util";

describe("computePlotMargins", () => {
    it("gives a sparkline the same small margin on every side", () => {
        const m = computePlotMargins(true, false);
        expect(m).toEqual({ marginTop: 4, marginRight: 4, marginBottom: 4, marginLeft: 4 });
        // A sparkline hides its axis (SingleLinePlot's `axis: !sparkline`), so `title`
        // never applies to one in practice — but the margin must not depend on it.
        expect(computePlotMargins(true, true)).toEqual(m);
    });

    it("gives a titled panel more headroom than an untitled one, everything else equal", () => {
        const titled = computePlotMargins(false, true);
        const untitled = computePlotMargins(false, false);
        expect(titled.marginTop).toBeGreaterThan(untitled.marginTop);
        expect(titled).toMatchObject({ marginRight: untitled.marginRight, marginBottom: untitled.marginBottom, marginLeft: untitled.marginLeft });
    });

    it("runs the chart flush against the pane's right edge", () => {
        for (const title of [true, false]) expect(computePlotMargins(false, title).marginRight).toBe(0);
    });

    it("never returns a non-positive top/bottom/left margin for a non-sparkline panel (tick text still needs room)", () => {
        for (const title of [true, false]) {
            const { marginTop, marginBottom, marginLeft } = computePlotMargins(false, title);
            for (const v of [marginTop, marginBottom, marginLeft]) expect(v).toBeGreaterThan(0);
        }
    });
});

describe("xTickFitsBeforeRightEdge", () => {
    // 100 s window across a 500 px wide plot with a 20 px left margin and no right margin.
    const fits = (secsFromRight: number) =>
        xTickFitsBeforeRightEdge(100_000 - secsFromRight * 1000, 0, 100_000, 500, 20, 0);

    it("keeps a tick label with room for half its width before the right edge", () => {
        expect(fits(50)).toBe(true);
        expect(fits(10)).toBe(true); // 10 s = 48 px from the edge
    });

    it("hides the label of a tick so close to the flush right edge that it would be clipped", () => {
        expect(fits(0)).toBe(false);
        expect(fits(2)).toBe(false); // ~10 px from the edge
    });

    it("a right margin makes room: the same tick fits once the plot stops short of the edge", () => {
        const d = 100_000 - 2000;
        expect(xTickFitsBeforeRightEdge(d, 0, 100_000, 500, 20, 40)).toBe(true);
    });

    it("leaves a degenerate domain alone", () => {
        expect(xTickFitsBeforeRightEdge(5, 5, 5, 500, 20, 0)).toBe(true);
    });
});

describe("computeLeftMarginForTicks", () => {
    it("leaves 1px between the plot's left edge and the widest y tick label (the pane adds its 1px border)", () => {
        expect(PLOT_EDGE_PAD_PX).toBe(1);
        // margin = edge pad + label width + the gutter Plot puts between a label and the axis
        expect(computeLeftMarginForTicks(18)).toBe(PLOT_EDGE_PAD_PX + 18 + PLOT_TICK_GUTTER_PX);
    });

    it("rounds a fractional width up, so the label never crosses the edge", () => {
        expect(computeLeftMarginForTicks(17.2)).toBe(PLOT_EDGE_PAD_PX + 18 + PLOT_TICK_GUTTER_PX);
    });

    it("grows with a wider label (e.g. a rate with units) and shrinks with a narrower one", () => {
        expect(computeLeftMarginForTicks(40)).toBeGreaterThan(computeLeftMarginForTicks(18));
        expect(computeLeftMarginForTicks(6)).toBeLessThan(computeLeftMarginForTicks(18));
    });

    it("falls back to the default left margin when nothing could be measured", () => {
        const fallback = computePlotMargins(false, false).marginLeft;
        for (const w of [0, NaN, -1]) expect(computeLeftMarginForTicks(w)).toBe(fallback);
    });
});

describe("buildPlotAxisLabelOptions", () => {
    it("turns both axis labels off: time and the unit are inferred from the ticks and title", () => {
        // `label: null` is how Plot suppresses an axis label; `undefined`
        // would let it fall back to the channel name ("time", "%", …).
        expect(buildPlotAxisLabelOptions()).toEqual({ x: { label: null }, y: { label: null } });
    });
});

// The remaining sysinfo-util exports had no test coverage anywhere in the repo
// before this file. Added here since it's the first test file for this module.

describe("resolveDomainBound", () => {
    it("passes a numeric bound straight through", () => {
        expect(resolveDomainBound(42, { ts: 0 })).toBe(42);
    });

    it("looks a string bound up as a field on the data item", () => {
        expect(resolveDomainBound("mem:total", { ts: 0, "mem:total": 16 })).toBe(16);
    });

    it("returns undefined for a string bound missing from the item, or an unsupported type", () => {
        expect(resolveDomainBound("mem:total", { ts: 0 })).toBeUndefined();
        expect(resolveDomainBound("mem:total", undefined as any)).toBeUndefined();
        expect(resolveDomainBound(undefined, { ts: 0 })).toBeUndefined();
    });
});

describe("getGapThresholdMs", () => {
    it("is 2.5x the interval, floored at 3000ms", () => {
        expect(getGapThresholdMs(1)).toBe(3000); // 2500 < floor
        expect(getGapThresholdMs(2)).toBe(5000); // 5000 > floor
    });

    it("treats a falsy interval as the 1.0s default", () => {
        expect(getGapThresholdMs(0)).toBe(getGapThresholdMs(1));
    });
});

describe("computeAutoMaxY", () => {
    const item = (x: number) => ({ ts: 0, x });

    it("pads the observed max by the padding fraction (default 15%)", () => {
        expect(computeAutoMaxY([item(10)], "x", 1, undefined)).toBeCloseTo(11.5);
    });

    it("never returns less than the floor, even with no data above it", () => {
        expect(computeAutoMaxY([item(0.1)], "x", 1, undefined)).toBe(1);
        expect(computeAutoMaxY([], "x", 1, undefined)).toBe(1);
    });

    it("never exceeds a hard cap, even when the padded value would clear it", () => {
        // observed 95 -> padded 109.25, but a hard cap (e.g. mem:total) must win.
        expect(computeAutoMaxY([item(95)], "x", 1, 100)).toBe(100);
    });

    it("ignores non-finite and non-numeric samples when finding the observed max", () => {
        const mixed = [item(5), { ts: 1, x: NaN }, { ts: 2, x: Infinity }, { ts: 3 }];
        expect(computeAutoMaxY(mixed, "x", 1, undefined)).toBeCloseTo(5.75);
    });

    it("honors a custom padding fraction", () => {
        expect(computeAutoMaxY([item(10)], "x", 1, undefined, 0.5)).toBeCloseTo(15);
    });
});
