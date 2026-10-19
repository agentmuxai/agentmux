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
        static pxRatio = 1;
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
        redraws: any[] = [];
        redraw(...args: any[]) {
            this.redraws.push(args);
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
import { SingleLinePlot, SysinfoZoomContext } from "./sysinfo-plot";
import type { DataItem } from "./sysinfo-types";

/** The size every chart is laid out at; a test can make it smaller. */
let observed = { width: 600, height: 200 };

class FixedSizeObserver {
    constructor(private cb: ResizeObserverCallback) {}
    observe(el: Element) {
        this.cb([{ target: el, contentRect: observed } as ResizeObserverEntry], this as any);
    }
    unobserve() {}
    disconnect() {}
}

beforeEach(() => {
    observed = { width: 600, height: 200 };
    made.charts.length = 0;
    vi.useFakeTimers();
    vi.stubGlobal("ResizeObserver", FixedSizeObserver);
});

afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.unstubAllGlobals();
});

/** Just enough of a uPlot instance for the time axis' label rule: `width` px of plot, 10 px per second from `t`. */
function plotArea(width: number) {
    const t = new Date(2026, 9, 13, 8, 52, 30).getTime() / 1000;
    return {
        bbox: { width },
        ctx: { font: "", measureText: () => ({ width: 40 }) },
        axes: [{ font: ["12px sans-serif"] }],
        valToPos: (v: number) => (v - t) * 10,
    };
}

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
        // Against the pane's edges: no gutter on the left, only the room its last time label needs on the right.
        expect(opts.padding).toEqual([7, 0, null, 1]);
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

    it("labels the time axis to the second on a short window, and the y axis with its unit", async () => {
        mount(() => series(sample(base, 40)));
        await vi.advanceTimersByTimeAsync(200);
        const [x, y] = made.charts[0].opts.axes;
        const t = new Date(2026, 9, 13, 8, 52, 30).getTime() / 1000;
        expect(x.values(plotArea(600), [t, t + 30], 0, 80, 30)).toEqual(["8:52:30", "8:53:00"]);
        expect(y.values(null, [0, 50, 100])).toEqual(["0%", "50%", "100%"]);
    });

    it("leaves a time label blank where it would cross the plot's right edge, keeping the others", async () => {
        mount(() => series(sample(base, 40)));
        await vi.advanceTimersByTimeAsync(200);
        const x = made.charts[0].opts.axes[0];
        const t = new Date(2026, 9, 13, 8, 52, 30).getTime() / 1000;
        // 600px wide, 10px per second, labels 40px wide: the tick at 590px would end at 610px.
        expect(x.values(plotArea(600), [t, t + 30, t + 59], 0, 80, 30)).toEqual(["8:52:30", "8:53:00", ""]);
        // A tick whose label fits (20px half-width + 1px inset) keeps it; one on the edge does not.
        expect(x.values(plotArea(600), [t + 57], 0, 80, 30)).toEqual(["8:53:27"]);
        expect(x.values(plotArea(600), [t + 60], 0, 80, 30)).toEqual([""]);
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

// Text zoom and placement (SPEC_SYSINFO_TEXT_ZOOM_DENSITY_AND_ONE_ZOOM_PATH_2026_10_18.md §4, §5).
describe("SingleLinePlot text zoom and placement", () => {
    const base = 1_791_900_000_000;
    type Extra = Partial<Parameters<typeof SingleLinePlot>[0]>;

    function mountWith(zoom: number, extra: Extra = {}) {
        return render(() => (
            <SysinfoZoomContext.Provider value={() => zoom}>
                <SingleLinePlot
                    plotData={series(sample(base, 40), sample(base + 1000, 42))}
                    yval="cpu"
                    yvalMeta={META}
                    blockId="b"
                    defaultColor="#a3e635"
                    targetLen={300}
                    intervalSecs={1}
                    {...extra}
                />
            </SysinfoZoomContext.Provider>
        ));
    }

    it("scales the axis font and the room text takes with the zoom, not the line", async () => {
        mountWith(1.5);
        await vi.advanceTimersByTimeAsync(200);
        const { opts } = made.charts[0];
        expect(opts.axes[0].font).toMatch(/^18px /);
        expect(opts.axes[1].font).toMatch(/^18px /);
        // Time axis: ceil(18 * 1.25) + tick 3 + gap 3; top padding: half a label + 1.
        expect(opts.axes[0].size).toBe(29);
        expect(opts.padding).toEqual([10, 0, null, 1]);
        expect(opts.axes[1].space).toBe(45);
        // The drawing itself doesn't zoom.
        expect(opts.series[1].width).toBe(2);
        expect(opts.axes[0].ticks.size).toBe(3);
    });

    it("draws ticks without labels where a neighbour shows them", async () => {
        mountWith(1, { timeAxis: false, yLabels: false });
        await vi.advanceTimersByTimeAsync(200);
        const [x, y] = made.charts[0].opts.axes;
        expect(x.size).toBe(4);
        expect(y.size).toBe(4);
        expect(x.values(null, [1, 2], 0, 80, 1)).toEqual(["", ""]);
        expect(y.values(null, [0, 50])).toEqual(["", ""]);
    });

    it("drops its axes when too short for them, keeping its title", async () => {
        observed = { width: 600, height: 40 };
        const { container } = mountWith(1);
        await vi.advanceTimersByTimeAsync(200);
        const { opts } = made.charts[0];
        expect(opts.axes).toEqual([{ show: false }, { show: false }]);
        expect(opts.padding).toBeUndefined();
        expect(container.querySelector(".sysinfo-chart-title")?.textContent).toMatch(/^CPU/);
    });

    it("reports a y-axis width of 0 when it has no y labels, so its column isn't kept wide", async () => {
        const reported: number[] = [];
        mountWith(1, { yLabels: false, onAxisWidth: (w) => reported.push(w) });
        await vi.advanceTimersByTimeAsync(200);
        expect(reported).toEqual([0]);
    });

    it("marks a top-row chart, so its title keeps clear of the floating buttons", async () => {
        const { container } = mountWith(1, { top: true });
        await vi.advanceTimersByTimeAsync(200);
        expect(container.querySelector(".sysinfo-chart")?.classList.contains("sysinfo-chart--top")).toBe(true);
    });

    it("reports its own y-axis width and takes its column's widest", async () => {
        const reported: number[] = [];
        mountWith(1, { axisWidth: 50, onAxisWidth: (w) => reported.push(w) });
        await vi.advanceTimersByTimeAsync(200);
        const y = made.charts[0].opts.axes[1];
        const self = {
            axes: [{}, { ticks: { size: 3 }, gap: 3, font: ["12px sans-serif"] }],
            ctx: { font: "", measureText: (t: string) => ({ width: t.length * 7 * devicePixelRatio }) },
        };
        // "100%": 4 × 7 + 6 = 34, under the column's 50.
        expect(y.size(self, ["0%", "100%"], 1, 0)).toBe(50);
        await Promise.resolve();
        expect(reported).toEqual([34]);
    });
});
