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
import { SingleLinePlot } from "./sysinfo-plot";
import { SysinfoZoomContext } from "./sysinfo-zoom";
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
        // Against the pane's edges: no gutter on either side, the y labels sit over the plot.
        expect(opts.padding).toEqual([7, 0, null, 0]);
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

    it("labels the time axis to the second on a short window", async () => {
        mount(() => series(sample(base, 40)));
        await vi.advanceTimersByTimeAsync(200);
        const [x, y] = made.charts[0].opts.axes;
        const t = new Date(2026, 9, 13, 8, 52, 30).getTime() / 1000;
        expect(x.values(plotArea(600), [t + 10, t + 40], 0, 80, 30)).toEqual(["8:52:40", "8:53:10"]);
        // The axis has no labels of its own: they are drawn over the plot (below).
        expect(y.values(null, [0, 50, 100])).toEqual(["", "", ""]);
    });

    it("leaves a time label blank where it would cross the plot's left or right edge, keeping the others", async () => {
        mount(() => series(sample(base, 40)));
        await vi.advanceTimersByTimeAsync(200);
        const x = made.charts[0].opts.axes[0];
        const t = new Date(2026, 9, 13, 8, 52, 30).getTime() / 1000;
        // 600px wide, 10px per second, labels 40px wide: the tick at 590px would end at 610px.
        expect(x.values(plotArea(600), [t + 3, t + 30, t + 59], 0, 80, 30)).toEqual(["8:52:33", "8:53:00", ""]);
        // The same at the left edge, where the plot now begins: a tick at 0 or 20px would be cut off at 20px of half-width.
        expect(x.values(plotArea(600), [t, t + 2], 0, 80, 30)).toEqual(["", ""]);
        expect(x.values(plotArea(600), [t + 3], 0, 80, 30)).toEqual(["8:52:33"]);
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
        expect(opts.padding).toEqual([10, 0, null, 0]);
        expect(opts.axes[1].space).toBe(45);
        // The drawing itself doesn't zoom.
        expect(opts.series[1].width).toBe(2);
        expect(opts.axes[0].ticks.size).toBe(3);
    });

    it("has no y labels where a neighbour shows them, and the plot still runs to the edge", async () => {
        mountWith(1, { timeAxis: false, yLabels: false });
        await vi.advanceTimersByTimeAsync(200);
        const [x, y] = made.charts[0].opts.axes;
        expect(x.size).toBe(4);
        expect(y.size).toBe(0);
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

    it("marks a top-row chart, so its title keeps clear of the floating buttons", async () => {
        const { container } = mountWith(1, { top: true });
        await vi.advanceTimersByTimeAsync(200);
        expect(container.querySelector(".sysinfo-chart")?.classList.contains("sysinfo-chart--top")).toBe(true);
    });

    /** Just enough of a uPlot instance for the y labels and the fade drawn after the plot. */
    function drawnPlot(erased: string[]) {
        const ctx = {
            font: "",
            globalCompositeOperation: "source-over",
            fillStyle: "",
            measureText: (t: string) => ({ width: t.length * 7 }),
            save() {},
            restore() {},
            createLinearGradient: () => ({ addColorStop() {} }),
            fillRect: (x: number, _y: number, w: number) => erased.push(`${x}+${w} ${ctx.globalCompositeOperation}`),
        };
        return {
            ctx,
            bbox: { left: 0, top: 0, width: 600, height: 100 },
            scales: { y: { min: 0, max: 100 } },
            axes: [{}, { font: ["12px sans-serif"] }],
            over: { offsetTop: 0 },
            valToPos: (v: number) => 100 - v,
        };
    }

    it("draws its y labels over the plot, and fades the plot's left end out under them", async () => {
        const { container } = mountWith(1);
        await vi.advanceTimersByTimeAsync(200);
        const { opts } = made.charts[0];
        // The axis only reports the ticks; no width, no tick marks, no labels of its own.
        expect(opts.axes[1].size).toBe(0);
        expect(opts.axes[1].ticks.show).toBe(false);
        opts.axes[1].values(null, [0, 50, 100]);
        const erased: string[] = [];
        opts.hooks.draw[0](drawnPlot(erased));
        const labels = [...container.querySelectorAll(".sysinfo-chart-ylabel")] as HTMLElement[];
        expect(labels.map((l) => l.textContent)).toEqual(["0%", "50%", "100%"]);
        expect(labels.map((l) => l.style.top)).toEqual(["100px", "50px", "0px"]);
        // The lowest hangs above its line; the others are centred on theirs.
        expect(labels.map((l) => l.style.transform)).toEqual(["translateY(-100%)", "translateY(-50%)", "translateY(-50%)"]);
        // Widest label "100%" is 28px; plus the 1px inset and the 8px margin: 37px of fade, from the edge.
        expect(erased).toEqual(["0+37 destination-out"]);
        // The title keeps to the right of the labels: the chart carries their width for it.
        const chartEl = container.querySelector(".sysinfo-chart") as HTMLElement;
        expect(chartEl.style.getPropertyValue("--sysinfo-ylabel-w")).toBe("29px");
    });

    it("draws neither labels nor fade where a neighbour has them", async () => {
        const { container } = mountWith(1, { yLabels: false });
        await vi.advanceTimersByTimeAsync(200);
        const { opts } = made.charts[0];
        opts.axes[1].values(null, [0, 50, 100]);
        const erased: string[] = [];
        opts.hooks.draw[0](drawnPlot(erased));
        expect(container.querySelectorAll(".sysinfo-chart-ylabel")).toHaveLength(0);
        expect(erased).toEqual([]);
        expect((container.querySelector(".sysinfo-chart") as HTMLElement).style.getPropertyValue("--sysinfo-ylabel-w")).toBe("");
    });
});
