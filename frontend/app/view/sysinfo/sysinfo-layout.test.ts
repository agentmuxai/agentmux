// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Where a Sysinfo chart's text goes (sysinfo-layout.ts).
 * docs/specs/SPEC_SYSINFO_TEXT_ZOOM_DENSITY_AND_ONE_ZOOM_PATH_2026_10_18.md §5.
 */
import { createRoot, createSignal } from "solid-js";
import { describe, expect, it } from "vitest";

import {
    axisFontPx,
    BARE_AXIS_PX,
    createAxisColumns,
    inTopRow,
    minFullChartPx,
    sharesPercentRange,
    showsTimeAxis,
    showsYLabels,
    timeAxisPx,
    topPaddingPx,
} from "./sysinfo-layout";

describe("text room at a zoom", () => {
    it("scales the axis font, the top padding and the time axis with the zoom", () => {
        expect(axisFontPx(1)).toBe(12);
        expect(axisFontPx(1.5)).toBe(18);
        expect(axisFontPx(0.5)).toBe(6);
        expect(topPaddingPx(1)).toBe(7);
        expect(topPaddingPx(2)).toBe(13);
        expect(timeAxisPx(1, true)).toBe(21);
        expect(timeAxisPx(2, true)).toBe(36);
        // Without labels a time axis is its tick marks, at any zoom.
        expect(timeAxisPx(2, false)).toBe(BARE_AXIS_PX);
    });

    it("needs room for 28px of plot (zoomed) plus its text before it shows axes", () => {
        expect(minFullChartPx(1, true)).toBe(28 + 7 + 21);
        expect(minFullChartPx(1, false)).toBe(28 + 7 + 4);
        // Zoomed out, a chart keeps its axes at a smaller height.
        expect(minFullChartPx(0.5, true)).toBeLessThan(minFullChartPx(1, true));
    });
});

describe("placement rules", () => {
    it("draws time labels on the bottom chart of each column only", () => {
        // 1 column of 3
        expect([0, 1, 2].map((i) => showsTimeAxis(i, 3, 1))).toEqual([false, false, true]);
        // 2 columns of 4: the last row, both columns
        expect([0, 1, 2, 3].map((i) => showsTimeAxis(i, 4, 2))).toEqual([false, false, true, true]);
        // 2 columns of 5: the left column ends with the lone chart in the last row,
        // the right column a row higher; both bottoms draw labels.
        expect([0, 1, 2, 3, 4].map((i) => showsTimeAxis(i, 5, 2))).toEqual([false, false, false, true, true]);
        // 2 columns of 3
        expect([0, 1, 2].map((i) => showsTimeAxis(i, 3, 2))).toEqual([false, true, true]);
    });

    it("draws y labels on the left column only when every chart shares a range", () => {
        expect([0, 1, 2, 3].map((i) => showsYLabels(i, 2, true))).toEqual([true, false, true, false]);
        expect([0, 1, 2, 3].map((i) => showsYLabels(i, 2, false))).toEqual([true, true, true, true]);
        expect(showsYLabels(0, 1, true)).toBe(true);
    });

    it("knows the top row", () => {
        expect([0, 1, 2].map((i) => inTopRow(i, 2))).toEqual([true, true, false]);
        expect([0, 1].map((i) => inTopRow(i, 1))).toEqual([true, false]);
    });

    it("treats the cores (all 0–100%) as one range, and mixed measures as not", () => {
        expect(sharesPercentRange(["cpu:0", "cpu:1", "cpu:2"])).toBe(true);
        expect(sharesPercentRange(["cpu", "mem:used"])).toBe(false);
        expect(sharesPercentRange(["cpu:0"])).toBe(false);
    });
});

describe("createAxisColumns", () => {
    it("gives each column its widest reported y axis, and forgets them for a new set of charts", () => {
        // Effects start once the root has returned, so the reset is driven from outside it.
        const { cols, setSet, dispose } = createRoot((dispose) => {
            const [set, setSet] = createSignal(["a"]);
            return { cols: createAxisColumns(() => set().join(",")), setSet, dispose };
        });
        cols.report(0, "a", 30);
        cols.report(0, "b", 44);
        cols.report(1, "c", 20);
        expect(cols.widthFor(0)).toBe(44);
        expect(cols.widthFor(1)).toBe(20);
        expect(cols.widthFor(2)).toBe(0);
        // A new array with the same charts (every sample makes one) keeps them.
        setSet(["a"]);
        expect(cols.widthFor(0)).toBe(44);
        setSet(["x"]);
        expect(cols.widthFor(0)).toBe(0);
        dispose();
    });
});
