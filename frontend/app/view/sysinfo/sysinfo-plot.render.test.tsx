// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The chart as drawn, not just its data: a gap in the samples must show as a
 * break in the line and the area, and the x ticks must tell neighbours apart.
 * docs/reports/REPORT_SYSINFO_REWRITE_AND_MODERN_MONITOR_2026_10_13.md section 2.
 */
import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { SingleLinePlot } from "./sysinfo-plot";
import { sampleReducer } from "./sysinfo-model";
import type { DataItem } from "./sysinfo-types";

const INTERVAL_SECS = 1;
const NUM_POINTS = 300;
const META = { name: "CPU", label: "%", color: "#a3e635", decimalPlaces: 0 } as TimeSeriesMeta;

/** jsdom has no layout: report a fixed size so the chart draws. */
class FixedSizeObserver {
    constructor(private cb: ResizeObserverCallback) {}
    observe(el: Element) {
        this.cb([{ target: el, contentRect: { width: 600, height: 200 } } as ResizeObserverEntry], this as any);
    }
    unobserve() {}
    disconnect() {}
}

beforeEach(() => {
    vi.stubGlobal("ResizeObserver", FixedSizeObserver);
});

afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
});

const sample = (ts: number, cpu: number): DataItem => ({ ts, cpu });

function series(...items: DataItem[]): DataItem[] {
    return items.reduce<DataItem[]>(
        (state, item) => sampleReducer(state, { type: "APPEND", item, intervalSecs: INTERVAL_SECS, numPoints: NUM_POINTS }),
        []
    );
}

function draw(data: DataItem[], targetLen = NUM_POINTS): HTMLElement {
    const { container } = render(() => (
        <SingleLinePlot
            plotData={data}
            yval="cpu"
            yvalMeta={META}
            blockId="b"
            defaultColor="#a3e635"
            targetLen={targetLen}
            intervalSecs={INTERVAL_SECS}
        />
    ));
    return container as HTMLElement;
}

/** The `d` of the first path in the group Plot draws for a mark. */
function pathOf(container: HTMLElement, mark: "area" | "line"): string {
    const group = container.querySelector(`[aria-label="${mark}"]`);
    return group?.querySelector("path")?.getAttribute("d") ?? "";
}

const runs = (d: string) => (d.match(/M/g) ?? []).length;

describe("SingleLinePlot with a gap in the samples", () => {
    const base = 1_791_900_000_000;
    // Three real samples, a 20 s silence, then three more.
    const gapped = () =>
        series(
            sample(base, 40),
            sample(base + 1000, 42),
            sample(base + 2000, 44),
            sample(base + 22_000, 80),
            sample(base + 23_000, 82),
            sample(base + 24_000, 84)
        );

    it("draws the line and the area as two runs, not one joined by a ramp", () => {
        const c = draw(gapped());
        expect(runs(pathOf(c, "line"))).toBe(2);
        expect(runs(pathOf(c, "area"))).toBe(2);
    });

    it("draws an unbroken series as one run", () => {
        const c = draw(series(sample(base, 40), sample(base + 1000, 42), sample(base + 2000, 44)));
        expect(runs(pathOf(c, "line"))).toBe(1);
        expect(runs(pathOf(c, "area"))).toBe(1);
    });

    it("never puts the area's edge on the baseline between the two runs", () => {
        const c = draw(gapped());
        const area = pathOf(c, "area");
        // Each run is a closed polygon from the baseline up to its values and back;
        // a ramp would be a third polygon whose top edge runs from the baseline.
        expect(area.split("Z").filter((s) => s.includes("M"))).toHaveLength(2);
    });
});

describe("SingleLinePlot x axis", () => {
    const start = new Date(2026, 9, 13, 8, 52, 0).getTime();
    const tickLabels = (c: HTMLElement) =>
        Array.from(c.querySelectorAll('[aria-label="x-axis tick label"] text'))
            .map((t) => t.textContent ?? "")
            .filter(Boolean);

    it("labels ticks to the second on a window of a couple of minutes, all different", () => {
        const data = series(...Array.from({ length: 120 }, (_, i) => sample(start + i * 1000, 40 + (i % 7))));
        const labels = tickLabels(draw(data, 120));
        expect(labels.length).toBeGreaterThanOrEqual(2);
        expect(new Set(labels).size).toBe(labels.length);
        for (const l of labels) expect(l).toMatch(/^\d{1,2}:\d{2}:\d{2}$/);
    });

    it("labels ticks by the minute on the default five-minute window, all different", () => {
        const data = series(...Array.from({ length: 300 }, (_, i) => sample(start + i * 1000, 40 + (i % 7))));
        const labels = tickLabels(draw(data));
        expect(labels.length).toBeGreaterThanOrEqual(2);
        expect(new Set(labels).size).toBe(labels.length);
        for (const l of labels) expect(l).toMatch(/^\d{1,2}:\d{2} (AM|PM)$/);
    });
});
