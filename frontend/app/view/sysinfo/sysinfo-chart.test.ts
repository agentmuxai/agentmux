// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import {
    formatValue,
    niceCeil,
    resolveCssColor,
    timeAxisLabels,
    toAlignedData,
    withAlpha,
    xWindowSecs,
    yRange,
} from "./sysinfo-chart";
import type { DataItem } from "./sysinfo-types";

const row = (ts: number, v: Record<string, number>): DataItem => ({ ts, ...v });
const marker = (ts: number): DataItem => ({ ts, cpu: NaN, blank: 1 });

describe("toAlignedData", () => {
    it("gives x in seconds and null for a gap marker or a value that isn't a number", () => {
        const rows = [row(1000, { cpu: 40 }), marker(1001), marker(19_999), row(20_000, { cpu: 80 }), row(21_000, {})];
        expect(toAlignedData(rows, ["cpu"])).toEqual([
            [1, 1.001, 19.999, 20, 21],
            [40, null, null, 80, null],
        ]);
    });

    it("lines several series up on the same x", () => {
        const [xs, a, b] = toAlignedData([row(1000, { a: 1, b: 2 }), row(2000, { a: 3, b: Infinity })], ["a", "b"]);
        expect(xs).toEqual([1, 2]);
        expect(a).toEqual([1, 3]);
        expect(b).toEqual([2, null]);
    });
});

describe("xWindowSecs", () => {
    it("ends at the newest sample and spans targetLen intervals", () => {
        expect(xWindowSecs([row(100_000, {}), row(300_000, {})], 120, 1)).toEqual([180, 300]);
        expect(xWindowSecs([], 120, 1)).toBeNull();
    });
});

describe("yRange", () => {
    const rows = [row(1000, { cpu: 30, net: 3.2, mem: 9, "mem:total": 32 })];

    it("keeps a percentage at 0 to 100", () => {
        expect(yRange(rows, "cpu", { maxy: 100 } as TimeSeriesMeta)).toEqual([0, 100]);
        expect(yRange(rows, "cpu", undefined)).toEqual([0, 100]);
    });

    it("scales a rate with no ceiling to its peak, rounded up to a value that reads well", () => {
        const [lo, hi] = yRange(rows, "net", { autoMaxY: true, autoMaxYFloor: 1 } as TimeSeriesMeta);
        expect(lo).toBe(0);
        expect(hi).toBe(5); // 3.2 plus 15 % headroom is 3.68, which rounds up to 5
    });

    it("never scales memory past its total", () => {
        const meta = { autoMaxY: true, autoMaxYFloor: 1, maxy: "mem:total" } as TimeSeriesMeta;
        expect(yRange([row(1000, { mem: 31.5, "mem:total": 32 })], "mem", meta)).toEqual([0, 32]);
    });

    it("never gives an empty range", () => {
        expect(yRange([row(1000, { x: 0 })], "x", { maxy: 0 } as TimeSeriesMeta)).toEqual([0, 1]);
    });
});

describe("niceCeil", () => {
    it("rounds up to 1, 2, 2.5 or 5 times a power of ten", () => {
        expect([0.3, 1, 1.1, 2.2, 3.68, 6, 99, 101, 0].map(niceCeil)).toEqual([0.5, 1, 2, 2.5, 5, 10, 100, 200, 1]);
    });
});

describe("labels and values", () => {
    it("timeAxisLabels shows seconds when ticks are under a minute apart", () => {
        const t = new Date(2026, 9, 13, 8, 52, 30).getTime() / 1000;
        expect(timeAxisLabels([t, t + 30], 30)).toEqual(["8:52:30", "8:53:00"]);
        expect(timeAxisLabels([t - 30], 60)).toEqual(["8:52 AM"]);
    });

    it("formatValue rounds, adds the unit, and shows a dash for no value", () => {
        expect(formatValue(42.345, 1, "%")).toBe("42.3%");
        expect(formatValue(null, 1, "%")).toBe("–");
        expect(formatValue(NaN, 0, "")).toBe("–");
    });
});

describe("colors for the canvas", () => {
    it("withAlpha takes hex and rgb colors and leaves anything else alone", () => {
        expect(withAlpha("#abc", 0.5)).toBe("rgba(170, 187, 204, 0.5)");
        expect(withAlpha("#419fe0", 0)).toBe("rgba(65, 159, 224, 0)");
        expect(withAlpha("rgb(65, 159, 224)", 0.7)).toBe("rgba(65, 159, 224, 0.7)");
        expect(withAlpha("rgba(1,2,3,0.4)", 1)).toBe("rgba(1, 2, 3, 1)");
        expect(withAlpha("red", 0.5)).toBe("red");
    });

    it("resolveCssColor reads a CSS variable from the element, with its fallback", () => {
        const el = document.createElement("div");
        el.style.setProperty("--accent-color", "rgb(65, 159, 224)");
        document.body.appendChild(el);
        expect(resolveCssColor(el, "var(--accent-color)")).toBe("rgb(65, 159, 224)");
        expect(resolveCssColor(el, "var(--not-set, #123)")).toBe("#123");
        expect(resolveCssColor(el, "#fff")).toBe("#fff");
        el.remove();
    });
});
