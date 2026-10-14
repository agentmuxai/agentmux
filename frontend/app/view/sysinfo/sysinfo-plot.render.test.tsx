// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The chart component against a recording stand-in for uPlot (jsdom has no
 * canvas): it is built with gaps as breaks and a time axis, is given `null` at a
 * gap, and takes new data by `setData` instead of being built again.
 * docs/reports/REPORT_SYSINFO_REWRITE_AND_MODERN_MONITOR_2026_10_13.md section 5.2.
 */
import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const made = vi.hoisted(() => ({ charts: [] as any[] }));

vi.mock("uplot", () => {
    class FakeUPlot {
        opts: any;
        data: any;
        setDataCalls: any[] = [];
        sizes: any[] = [];
        destroyed = false;
        constructor(opts: any, data: any, el: HTMLElement) {
            this.opts = opts;
            this.data = data;
            el.appendChild(document.createElement("canvas"));
            made.charts.push(this);
        }
        setData(d: any) {
            this.data = d;
            this.setDataCalls.push(d);
        }
        setSize(s: any) {
            this.sizes.push(s);
        }
        destroy() {
            this.destroyed = true;
        }
    }
    return { default: FakeUPlot };
});
vi.mock("uplot/dist/uPlot.min.css", () => ({}));

import { markWindowThemeApplied } from "@/app/store/window-appearance";
import { sampleReducer } from "./sysinfo-model";
import { SingleLinePlot } from "./sysinfo-plot";
import type { DataItem } from "./sysinfo-types";

class FixedSizeObserver {
    constructor(private cb: ResizeObserverCallback) {}
    observe(el: Element) {
        this.cb([{ target: el, contentRect: { width: 600, height: 200 } } as ResizeObserverEntry], this as any);
    }
    unobserve() {}
    disconnect() {}
}

beforeEach(() => {
    made.charts.length = 0;
    vi.useFakeTimers();
    vi.stubGlobal("ResizeObserver", FixedSizeObserver);
});

afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.unstubAllGlobals();
});

const META = { name: "CPU", label: "%", color: "#a3e635", decimalPlaces: 0, maxy: 100 } as TimeSeriesMeta;
const sample = (ts: number, cpu: number): DataItem => ({ ts, cpu });
const series = (...items: DataItem[]) =>
    items.reduce<DataItem[]>((s, item) => sampleReducer(s, { type: "APPEND", item, intervalSecs: 1, numPoints: 300 }), []);

function mount(data: () => DataItem[]) {
    return render(() => (
        <SingleLinePlot
            plotData={data()}
            yval="cpu"
            yvalMeta={META}
            blockId="b"
            defaultColor="#a3e635"
            targetLen={300}
            intervalSecs={1}
            syncKey="sysinfo-b"
        />
    ));
}

describe("SingleLinePlot on uPlot", () => {
    const base = 1_791_900_000_000;

    it("is built once its size is known, with gaps as breaks, a time x axis and a shared crosshair", async () => {
        mount(() => series(sample(base, 40), sample(base + 1000, 42)));
        await vi.advanceTimersByTimeAsync(200);
        expect(made.charts).toHaveLength(1);
        const { opts } = made.charts[0];
        expect(opts.series[1].spanGaps).toBe(false);
        expect(opts.scales.x.time).toBe(true);
        expect(opts.cursor.sync.key).toBe("sysinfo-b");
        // Only the time is shared between the pane's charts, never a y value.
        expect(opts.cursor.sync.scales).toEqual(["x", null]);
        expect([opts.width, opts.height]).toEqual([600, 200]);
        // Fixed sides, so the charts and the core heatmap in one pane line up.
        expect(opts.padding).toEqual([null, 36, null, 0]);
        expect(opts.scales.y.range()).toEqual([0, 100]);
        // The x window ends at the newest sample and spans five minutes.
        expect(opts.scales.x.range()).toEqual([(base + 1000) / 1000 - 300, (base + 1000) / 1000]);
    });

    it("hands uPlot null at a gap, so the line and area break there", async () => {
        mount(() => series(sample(base, 40), sample(base + 1000, 42), sample(base + 25_000, 90)));
        await vi.advanceTimersByTimeAsync(200);
        const [xs, ys] = made.charts[0].data;
        expect(xs.length).toBe(ys.length);
        expect(ys).toEqual([40, 42, null, null, 90]);
    });

    it("takes new samples with setData, not a new chart", async () => {
        const [data, setData] = createSignal(series(sample(base, 40)));
        mount(data);
        await vi.advanceTimersByTimeAsync(200);
        setData(series(sample(base, 40), sample(base + 1000, 55)));
        setData(series(sample(base, 40), sample(base + 1000, 55), sample(base + 2000, 60)));
        expect(made.charts).toHaveLength(1);
        const calls = made.charts[0].setDataCalls;
        expect(calls.length).toBe(2);
        expect(calls[1][1]).toEqual([40, 55, 60]);
    });

    it("labels the time axis to the second on a short window, and the y axis without a unit", async () => {
        mount(() => series(sample(base, 40)));
        await vi.advanceTimersByTimeAsync(200);
        const [x, y] = made.charts[0].opts.axes;
        const t = new Date(2026, 9, 13, 8, 52, 30).getTime() / 1000;
        expect(x.values(null, [t, t + 30], 0, 80, 30)).toEqual(["8:52:30", "8:53:00"]);
        expect(y.values(null, [0, 50, 100])).toEqual(["0", "50", "100"]);
    });

    it("is built again when the theme changes, so its canvas takes the new colors", async () => {
        mount(() => series(sample(base, 40)));
        await vi.advanceTimersByTimeAsync(200);
        markWindowThemeApplied("monokai");
        expect(made.charts).toHaveLength(2);
        expect(made.charts[0].destroyed).toBe(true);
        markWindowThemeApplied("default");
    });

    it("is destroyed with the pane", async () => {
        const { unmount } = mount(() => series(sample(base, 40)));
        await vi.advanceTimersByTimeAsync(200);
        unmount();
        expect(made.charts[0].destroyed).toBe(true);
    });
});
