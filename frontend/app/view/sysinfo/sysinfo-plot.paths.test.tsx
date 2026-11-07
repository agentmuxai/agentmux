// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/* eslint-disable @typescript-eslint/no-explicit-any -- the uPlot stand-in and its fake chart hold untyped options */

// The Sysinfo time chart (TimeChart and SingleLinePlot in sysinfo-plot.tsx), path by path, beside
// sysinfo-plot.render.test.tsx: what it hands uPlot (colours, grid, ticks, fonts, series, cursor), when it
// draws bare, the time labels at the plot's edges, the y labels drawn over the plot and the fade under them, the tooltip's text and place,
// when a change is a new chart and when it is only a new size or new data, and what SingleLinePlot passes
// down. uPlot is a recording stand-in (jsdom has no canvas); colours are tagged so they can be read back.

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const made = vi.hoisted(() => ({ charts: [] as any[] }));
const fadeLeftEnd = vi.hoisted(() => vi.fn());

vi.mock("uplot", () => {
    class FakeUPlot {
        static pxRatio = 2;
        opts: any;
        data: any;
        setDataCalls: any[] = [];
        sizes: any[] = [];
        redraws: any[] = [];
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
vi.mock("./sysinfo-chart", async (orig) => {
    const real = await orig<typeof import("./sysinfo-chart")>();
    return {
        ...real,
        fadeLeftEnd: fadeLeftEnd,
        resolveCssColor: (_el: Element, c: string) => `R(${c})`,
        withAlpha: (c: string, a: number) => `A(${c},${a})`,
    };
});

import { sampleReducer } from "./sysinfo-model";
import { figures, titled, unitFormat } from "./sysinfo-overview-data";
import { SingleLinePlot, TimeChart, type TimeChartProps } from "./sysinfo-plot";
import type { DataItem } from "./sysinfo-types";
import { SysinfoZoomContext } from "./sysinfo-zoom";

const observers: { cb: ResizeObserverCallback; observed: Element[]; disconnected: number }[] = [];

class RecordingObserver {
    rec: (typeof observers)[number];
    constructor(cb: ResizeObserverCallback) {
        this.rec = { cb, observed: [], disconnected: 0 };
        observers.push(this.rec);
    }
    observe(el: Element) {
        this.rec.observed.push(el);
    }
    unobserve() {}
    disconnect() {
        this.rec.disconnected++;
    }
}

beforeEach(() => {
    fadeLeftEnd.mockClear();
    made.charts.length = 0;
    observers.length = 0;
    vi.useFakeTimers();
    vi.stubGlobal("ResizeObserver", RecordingObserver);
});

afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
});

const BASE = 1_791_900_000_000;
const sample = (ts: number, cpu: number): DataItem => ({ ts, cpu });
const rowsOf = (...items: DataItem[]) =>
    items.reduce<DataItem[]>(
        (s, item) => sampleReducer(s, { type: "APPEND", item, intervalSecs: 1, numPoints: 300 }),
        []
    );
const ROWS = rowsOf(sample(BASE, 40), sample(BASE + 1000, 42));
const SPEC = { key: "cpu", label: "CPU", color: "#a3e635" };
const SPEC2 = { key: "mem", label: "Mem", color: "#38bdf8" };

/** Sets the size the observer reports, as the pane would lay the chart out. */
function layout(width: number, height: number, entries = 1) {
    const rec = observers[observers.length - 1];
    const list = Array.from({ length: entries }, (_, i) =>
        i === entries - 1 ? { contentRect: { width, height } } : { contentRect: { width: 1, height: 1 } }
    );
    rec.cb(list as unknown as ResizeObserverEntry[], {} as ResizeObserver);
}

const last = () => made.charts[made.charts.length - 1];

type Extra = Partial<TimeChartProps>;

/** A chart whose changing inputs are signals, laid out at 600x200 unless it says otherwise. */
function mountChart(init: Extra = {}, size: [number, number] | null = [600, 200], zoomStart = 1) {
    const [series, setSeries] = createSignal(init.series ?? [SPEC]);
    const [plotData, setPlotData] = createSignal<DataItem[]>("plotData" in init ? (init.plotData as DataItem[]) : ROWS);
    const [targetLen, setTargetLen] = createSignal(init.targetLen ?? 300);
    const [intervalSecs, setIntervalSecs] = createSignal(init.intervalSecs ?? 1);
    const [timeAxis, setTimeAxis] = createSignal(init.timeAxis);
    const [yLabels, setYLabels] = createSignal(init.yLabels);
    const [zoom, setZoom] = createSignal(zoomStart);
    const yRangeFn = init.yRange ?? vi.fn(() => [3, 9] as [number, number]);
    const result = render(() => (
        <SysinfoZoomContext.Provider value={zoom}>
            <TimeChart
                plotData={plotData()}
                series={series()}
                yRange={yRangeFn}
                format={init.format ?? ((v) => `${v}u`)}
                axisFormat={init.axisFormat}
                targetLen={targetLen()}
                intervalSecs={intervalSecs()}
                syncKey={init.syncKey}
                sparkline={init.sparkline}
                timeAxis={timeAxis()}
                yLabels={yLabels()}
                top={init.top}
                title={init.title}
                class={init.class}
            />
        </SysinfoZoomContext.Provider>
    ));
    if (size) layout(size[0], size[1]);
    return {
        ...result,
        yRangeFn,
        root: result.container.querySelector(".sysinfo-chart") as HTMLElement,
        set: { setSeries, setPlotData, setTargetLen, setIntervalSecs, setTimeAxis, setYLabels, setZoom },
    };
}

describe("TimeChart: the time labels at the plot's edge", () => {
    const T = new Date(2026, 9, 13, 8, 52, 30).getTime() / 1000;
    // pxRatio is 2: a 200px bbox is a 100px plot; a 40px label has a half-width of 10.
    const area = (width: number, valToPos: (v: number, s: string) => number) => ({
        bbox: { width },
        ctx: { font: "", measureText: () => ({ width: 40 }) },
        axes: [{ font: ["12px a"] }, { font: ["13px b"] }],
        valToPos,
    });
    const labels = (u: any, splits: number[]) => {
        mountChart();
        return last().opts.axes[0].values(u, splits, 0, 80, 30) as string[];
    };

    it("keeps a label that ends exactly at the edge, less the 1px inset, and blanks the next pixel", () => {
        const at = (px: number) =>
            labels(
                area(200, (_v, s) => (s === "x" ? px : -1000)),
                [T]
            );
        expect(at(89)[0]).not.toBe("");
        cleanup();
        expect(at(90)[0]).toBe("");
    });

    it("keeps a label that starts exactly at the left edge, less the 1px inset, and blanks the next pixel", () => {
        const at = (px: number) =>
            labels(
                area(200, (_v, s) => (s === "x" ? px : -1000)),
                [T]
            );
        // 11 - 10 - 1 = 0.
        expect(at(11)[0]).not.toBe("");
        cleanup();
        expect(at(10)[0]).toBe("");
    });

    it("measures where a tick is, along x, for each tick", () => {
        const out = labels(
            area(200, (v, s) => (s === "x" ? (v - T) * 10 : -1000)),
            [T + 3, T + 9.5]
        );
        expect(out[0]).not.toBe("");
        expect(out[1]).toBe("");
    });

    it("measures the label in the time axis' font, once the font is set", () => {
        const u = area(200, () => 0);
        labels(u, [T]);
        expect(u.ctx.font).toBe("12px a");
    });

    it("leaves every label as it is when the plot has no width", () => {
        const out = labels(
            area(0, () => 500),
            [T, T + 30]
        );
        expect(out.every((l) => l !== "")).toBe(true);
    });
});

describe("TimeChart: what it hands uPlot", () => {
    it("tags the axes, grid and ticks with the muted colour, and reads it from the theme", () => {
        mountChart();
        const [x, y] = last().opts.axes;
        const muted = "R(var(--grey-text-color))";
        expect(x.stroke).toBe(muted);
        expect(y.stroke).toBe(muted);
        for (const a of [x, y]) expect(a.grid).toEqual({ stroke: `A(${muted},0.25)`, width: 1 });
        expect(x.ticks).toEqual({ stroke: `A(${muted},0.4)`, width: 1, size: 3 });
        expect(x.gap).toBe(3);
    });

    it("sets the axis font from the zoom and the pane's font family, or sans-serif without one", () => {
        vi.spyOn(window, "getComputedStyle").mockReturnValue({ fontFamily: "Inter" } as CSSStyleDeclaration);
        mountChart();
        expect(last().opts.axes[0].font).toBe("12px Inter");
        expect(last().opts.axes[1].font).toBe("12px Inter");
        cleanup();
        vi.spyOn(window, "getComputedStyle").mockReturnValue({ fontFamily: "" } as CSSStyleDeclaration);
        mountChart();
        expect(last().opts.axes[0].font).toBe("12px sans-serif");
    });

    it("spaces the labels by the zoom, rounded", () => {
        mountChart({}, [600, 400], 1.5);
        const [x, y] = last().opts.axes;
        expect(x.space).toBe(120);
        expect(y.space).toBe(45);
        cleanup();
        mountChart({}, [600, 400], 1);
        expect([last().opts.axes[0].space, last().opts.axes[1].space]).toEqual([80, 30]);
        cleanup();
        mountChart({}, [600, 400], 1.3);
        expect([last().opts.axes[0].space, last().opts.axes[1].space]).toEqual([104, 39]);
    });

    it("shows no legend, and a crosshair that can't be dragged, with a 6px point", () => {
        mountChart();
        const { opts } = last();
        expect(opts.legend).toEqual({ show: false });
        expect(opts.cursor.drag).toEqual({ x: false, y: false });
        expect(opts.cursor.points).toEqual({ size: 6 });
    });

    it("shares no crosshair without a key", () => {
        mountChart();
        expect(last().opts.cursor.sync).toBeUndefined();
        cleanup();
        mountChart({ syncKey: "k" });
        expect(last().opts.cursor.sync).toEqual({ key: "k", scales: ["x", null] });
    });

    it("gives the x scale time, in the window of the data, and the y scale the range of the rows", () => {
        const { yRangeFn } = mountChart();
        const { scales } = last().opts;
        expect(scales.x.time).toBe(true);
        expect(scales.x.range()).toEqual([(BASE + 1000) / 1000 - 300, (BASE + 1000) / 1000]);
        expect(scales.y.range()).toEqual([3, 9]);
        expect(yRangeFn).toHaveBeenCalledWith(ROWS);
    });

    it("starts the x scale at 0..1 with no data, and reads the rows as empty when there are none", () => {
        const { yRangeFn } = mountChart({ plotData: undefined as unknown as DataItem[] });
        const { scales } = last().opts;
        expect(scales.x.range()).toEqual([0, 1]);
        expect(scales.y.range()).toEqual([3, 9]);
        expect(yRangeFn).toHaveBeenCalledWith([]);
    });

    it("hands uPlot the series' data, in the order of its series", () => {
        mountChart({ series: [SPEC, SPEC2], plotData: rowsOf({ ts: BASE, cpu: 40, mem: 7 } as DataItem) });
        expect(last().data[1]).toEqual([40]);
        expect(last().data[2]).toEqual([7]);
    });

    it("draws each series in its own colour, labelled, 2px wide, with its points off and its area filled to 0", () => {
        mountChart({ series: [SPEC, SPEC2] });
        const { series } = last().opts;
        expect(series[0]).toEqual({});
        expect(series).toHaveLength(3);
        expect(series[1]).toMatchObject({
            label: "CPU",
            stroke: "R(#a3e635)",
            width: 2,
            spanGaps: false,
            points: { show: false },
            fillTo: 0,
        });
        expect(series[2]).toMatchObject({ label: "Mem", stroke: "R(#38bdf8)" });
    });

    it("fills several series with a flat wash, and a lone one with a gradient from 70% to nothing", () => {
        mountChart({ series: [SPEC, SPEC2] });
        expect(last().opts.series[1].fill).toBe("A(R(#a3e635),0.25)");
        expect(last().opts.series[2].fill).toBe("A(R(#38bdf8),0.25)");
        cleanup();
        mountChart();
        const fill = last().opts.series[1].fill;
        expect(typeof fill).toBe("function");
        const gradient = { addColorStop: vi.fn() };
        const ctx = { createLinearGradient: vi.fn(() => gradient) };
        expect(fill({ ctx, bbox: { top: 10, height: 100 } })).toBe(gradient);
        expect(ctx.createLinearGradient).toHaveBeenCalledWith(0, 10, 0, 110);
        expect(gradient.addColorStop.mock.calls).toEqual([
            [0, "A(R(#a3e635),0.7)"],
            [1, "A(R(#a3e635),0)"],
        ]);
    });

    it("draws a sparkline's line thinner, and without axes", () => {
        mountChart({ sparkline: true });
        const { opts } = last();
        expect(opts.series[1].width).toBe(1.5);
        expect(opts.axes).toEqual([{ show: false }, { show: false }]);
        expect(opts.padding).toBeUndefined();
    });

    it("pads the top for the top label, and nothing on the left or the right", () => {
        mountChart();
        expect(last().opts.padding).toEqual([7, 0, null, 0]);
    });

    it("sizes the time axis for labels only if it has them", () => {
        mountChart();
        expect(last().opts.axes[0].size).toBe(21);
        expect(last().opts.axes[0].values({ bbox: { width: 0 } }, [1, 2], 0, 80, 1)).toHaveLength(2);
        cleanup();
        mountChart({ timeAxis: false });
        expect(last().opts.axes[0].size).toBe(4);
        expect(last().opts.axes[0].values({ bbox: { width: 0 } }, [1, 2], 0, 80, 1)).toEqual(["", ""]);
    });

    it("has a y axis with no width, no gap and no ticks, that only draws the grid and writes no labels", () => {
        mountChart();
        const y = last().opts.axes[1];
        expect(y).toMatchObject({ size: 0, gap: 0, ticks: { show: false }, stroke: "R(var(--grey-text-color))" });
        expect(y.grid).toEqual({ stroke: "A(R(var(--grey-text-color)),0.25)", width: 1 });
        expect(y.values(null, [0, 50])).toEqual(["", ""]);
    });
});

/** Just enough of a uPlot instance for the labels drawn over the plot. */
function fakeU(over: { min?: number; max?: number } = {}) {
    return {
        ctx: { font: "", measureText: vi.fn((t: string) => ({ width: t.length * 10 + 1 })) },
        axes: [{ font: ["12px x"] }, { font: ["13px y"] }],
        scales: { y: { min: over.min, max: over.max } },
        over: { offsetTop: 4 },
        bbox: { left: 0, top: 0, width: 100, height: 100 },
        valToPos: vi.fn((v: number, s: string) => (s === "y" ? 100 - v : 999)),
    };
}

describe("TimeChart: the y labels over the plot", () => {
    const WIDTH_VAR = "--sysinfo-ylabel-w";

    function labelled(init: Extra = {}, size: [number, number] | null = [600, 200]) {
        const mounted = mountChart(init, size);
        const chart = last();
        const yaxis = mounted.root.querySelector(".sysinfo-chart-yaxis") as HTMLElement;
        const draw = (u = fakeU({ min: 0, max: 100 })) => chart.opts.hooks.draw[0](u);
        const ticks = (splits: number[]) => chart.opts.axes[1].values(null, splits);
        const shown = () => [...yaxis.children] as HTMLElement[];
        return { ...mounted, chart, yaxis, draw, ticks, shown };
    }

    it("draws a label for each tick the axis reported, in the format, over the plot", () => {
        const { ticks, draw, shown, yaxis } = labelled();
        ticks([0, 50, 100]);
        draw();
        expect(shown().map((e) => e.textContent)).toEqual(["0u", "50u", "100u"]);
        expect(shown().every((e) => e.className === "sysinfo-chart-ylabel")).toBe(true);
        expect(yaxis.className).toBe("sysinfo-chart-yaxis");
    });

    it("puts each at its value, below the chart's top, and the lowest above its line", () => {
        const { ticks, draw, shown } = labelled();
        ticks([0, 50, 100]);
        draw();
        // 4 (the plot's top) + 100 - value.
        expect(shown().map((e) => e.style.top)).toEqual(["104px", "54px", "4px"]);
        expect(shown().map((e) => e.style.transform)).toEqual([
            "translateY(-100%)",
            "translateY(-50%)",
            "translateY(-50%)",
        ]);
    });

    it("uses the axis format if there is one, and never writes a minus sign", () => {
        const { ticks, draw, shown } = labelled({ axisFormat: (v) => `${v}a` });
        ticks([-20, 0, 20]);
        draw(fakeU({ min: -20, max: 20 }));
        expect(shown().map((e) => e.textContent)).toEqual(["20a", "0a", "20a"]);
        cleanup();
        const plain = labelled();
        plain.ticks([-20, 0, 20]);
        plain.draw(fakeU({ min: -20, max: 20 }));
        expect(plain.shown().map((e) => e.textContent)).toEqual(["20u", "0u", "20u"]);
    });

    it("draws only the ticks inside the scale, and reads a scale with no limits as 0 to 1", () => {
        const { ticks, draw, shown } = labelled();
        ticks([0, 50, 100]);
        draw(fakeU({ min: 10, max: 60 }));
        expect(shown().map((e) => e.textContent)).toEqual(["50u"]);
        expect(shown()[0].style.transform).toBe("translateY(-100%)");
        ticks([0, 1, 5]);
        draw(fakeU());
        expect(shown().map((e) => e.textContent)).toEqual(["0u", "1u"]);
    });

    it("measures in the y axis' font, and fades the plot's left end past the widest label", () => {
        const { ticks, draw } = labelled();
        ticks([0, 50, 100]);
        const u = fakeU({ min: 0, max: 100 });
        draw(u);
        expect(u.ctx.font).toBe("13px y");
        // The widest is "100u": 41px at a pixel ratio of 2 is 20.5; plus the 1px inset and 8px of fade, in canvas pixels.
        expect(fadeLeftEnd).toHaveBeenCalledTimes(1);
        expect(fadeLeftEnd).toHaveBeenCalledWith(u, (20.5 + 1 + 8) * 2);
    });

    it("tells the chart how wide its labels are, so the title keeps to their right", () => {
        const { ticks, draw, root } = labelled();
        ticks([0, 50, 100]);
        draw();
        // ceil(20.5) + the 1px inset.
        expect(root.style.getPropertyValue(WIDTH_VAR)).toBe("22px");
    });

    it("draws nothing before the axis has reported its ticks", () => {
        const { draw, shown, root } = labelled();
        root.style.setProperty(WIDTH_VAR, "9px");
        draw();
        expect(shown()).toHaveLength(0);
        expect(root.style.getPropertyValue(WIDTH_VAR)).toBe("");
        expect(fadeLeftEnd).not.toHaveBeenCalled();
    });

    it("clears its labels and its width when no tick is left inside the scale", () => {
        const { ticks, draw, shown, root } = labelled();
        ticks([0, 50, 100]);
        draw();
        expect(shown()).toHaveLength(3);
        draw(fakeU({ min: 200, max: 300 }));
        expect(shown()).toHaveLength(0);
        expect(root.style.getPropertyValue(WIDTH_VAR)).toBe("");
        expect(fadeLeftEnd).toHaveBeenCalledTimes(1);
    });

    it("replaces its labels each time, not adds to them", () => {
        const { ticks, draw, shown } = labelled();
        ticks([0, 50, 100]);
        draw();
        draw();
        expect(shown()).toHaveLength(3);
    });

    it("draws none when it has no y labels, even with ticks reported", () => {
        const { ticks, draw, shown, root } = labelled({ yLabels: false });
        root.style.setProperty(WIDTH_VAR, "9px");
        ticks([0, 50, 100]);
        draw();
        expect(shown()).toHaveLength(0);
        expect(root.style.getPropertyValue(WIDTH_VAR)).toBe("");
        expect(fadeLeftEnd).not.toHaveBeenCalled();
    });

    it("draws none once the chart goes bare, though the ticks it reported are still known", async () => {
        const { ticks, root, yaxis } = labelled();
        ticks([0, 50, 100]);
        await vi.advanceTimersByTimeAsync(150);
        layout(600, 30);
        const bare = last();
        expect(bare.opts.axes).toEqual([{ show: false }, { show: false }]);
        root.style.setProperty(WIDTH_VAR, "9px");
        bare.opts.hooks.draw[0](fakeU({ min: 0, max: 100 }));
        expect(yaxis.children).toHaveLength(0);
        expect(root.style.getPropertyValue(WIDTH_VAR)).toBe("");
        expect(fadeLeftEnd).not.toHaveBeenCalled();
    });

    it("draws none on a sparkline", () => {
        const { draw, shown } = labelled({ sparkline: true });
        draw();
        expect(shown()).toHaveLength(0);
        expect(fadeLeftEnd).not.toHaveBeenCalled();
    });
});

describe("TimeChart: when it draws bare", () => {
    it("keeps its axes at and above the least height, and drops them below it", () => {
        // 28 (plot) + 7 (top padding) + 21 (a time axis) at 100%.
        mountChart({}, [600, 56]);
        expect(last().opts.axes[0].show).not.toBe(false);
        cleanup();
        made.charts.length = 0;
        mountChart({}, [600, 55]);
        expect(last().opts.axes).toEqual([{ show: false }, { show: false }]);
    });

    it("asks less height of a chart with no time labels", () => {
        // 28 + 7 + 4.
        mountChart({ timeAxis: false }, [600, 45]);
        expect(last().opts.axes[0].show).not.toBe(false);
        cleanup();
        mountChart({}, [600, 45]);
        expect(last().opts.axes[0].show).toBe(false);
    });

    it("asks more height at a higher zoom", () => {
        // 42 + 10 + 29 at 150%.
        mountChart({}, [600, 70], 1.5);
        expect(last().opts.axes[0].show).toBe(false);
        cleanup();
        mountChart({}, [600, 70], 1);
        expect(last().opts.axes[0].show).not.toBe(false);
        cleanup();
        mountChart({}, [600, 81], 1.5);
        expect(last().opts.axes[0].show).not.toBe(false);
    });

    it("draws a sparkline bare at any height", () => {
        mountChart({ sparkline: true }, [600, 400]);
        expect(last().opts.axes[0].show).toBe(false);
    });

    it("is not built before its size is known, and not built again for the first size", () => {
        mountChart({}, null);
        expect(made.charts).toHaveLength(0);
        layout(600, 200);
        expect(made.charts).toHaveLength(1);
        expect(last().sizes).toEqual([]);
    });
});

describe("TimeChart: the tooltip", () => {
    function tooltip(init: Extra = {}) {
        const { container, root } = mountChart(init);
        const tip = root.querySelector(".sysinfo-chart-tip") as HTMLElement;
        Object.defineProperty(tip, "offsetWidth", { configurable: true, value: 60 });
        const canvas = container.querySelector(".sysinfo-chart-canvas") as HTMLElement;
        Object.defineProperty(canvas, "clientWidth", { configurable: true, value: 200 });
        const valToPos = vi.fn((v: number, s: string) => (s === "y" ? v * 2 : 999));
        const hook = last().opts.hooks.setCursor[0] as (u: any) => void;
        const show = (
            u: Partial<{ idx: number | null; left: number | null; data: any[]; offsetLeft: number; offsetTop: number }>
        ) =>
            hook({
                cursor: { idx: "idx" in u ? u.idx : 0, left: "left" in u ? u.left : 0 },
                data: u.data ?? [
                    [T0, T1],
                    [10, -20],
                ],
                over: { offsetLeft: u.offsetLeft ?? 0, offsetTop: u.offsetTop ?? 0 },
                valToPos,
            });
        return { tip, show, valToPos };
    }
    const T0 = 1_791_900_000;
    const T1 = T0 + 30;
    const at = (t: number) => new Date(t * 1000).toLocaleTimeString();

    it("starts hidden", () => {
        const { tip } = tooltip();
        expect(tip.style.display).toBe("none");
    });

    it("shows the time and the value at the cursor, in the format, with no sign", () => {
        const { tip, show } = tooltip();
        show({ idx: 1 });
        expect(tip.style.display).toBe("block");
        expect(tip.textContent).toBe(`${at(T1)}  20u`);
        show({ idx: 0 });
        expect(tip.textContent).toBe(`${at(T0)}  10u`);
    });

    it("names each series when there are several, and leaves out the ones with no sample there", () => {
        const data = [
            [T0, T1],
            [10, null],
            [-5, 7],
        ];
        const { tip, show } = tooltip({ series: [SPEC, SPEC2] });
        show({ idx: 0, data });
        expect(tip.textContent).toBe(`${at(T0)}  CPU 10u  Mem 5u`);
        show({ idx: 1, data });
        expect(tip.textContent).toBe(`${at(T1)}  Mem 7u`);
    });

    it("keeps no name for a lone series, even a lone one with a sample missing", () => {
        const { tip, show } = tooltip();
        show({ idx: 0 });
        expect(tip.textContent).not.toContain("CPU");
    });

    it("hides when the cursor is off the data, or every series has no sample there", () => {
        const { tip, show } = tooltip({ series: [SPEC, SPEC2] });
        show({ idx: 0, data: [[T0], [10], [5]] });
        expect(tip.style.display).toBe("block");
        show({ idx: null });
        expect(tip.style.display).toBe("none");
        show({ idx: 0, data: [[T0], [10], [5]] });
        expect(tip.style.display).toBe("block");
        show({ idx: 0, data: [[T0], [null], [null]] });
        expect(tip.style.display).toBe("none");
    });

    it("never shows on a sparkline", () => {
        const { tip, show } = tooltip({ sparkline: true });
        show({ idx: 0 });
        expect(tip.style.display).toBe("none");
    });

    it("sits above the first sample's point, 30px up, and no higher than the chart's top", () => {
        const { tip, show, valToPos } = tooltip();
        show({ idx: 0, offsetTop: 50 });
        expect(valToPos.mock.calls[0]).toEqual([10, "y"]);
        // 50 + 20 - 30.
        expect(tip.style.top).toBe("40px");
        show({ idx: 1, offsetTop: 50 });
        // 50 - 40 - 30 is above the top.
        expect(tip.style.top).toBe("0px");
    });

    it("anchors on the first series' sample, not the last", () => {
        const { show, valToPos } = tooltip({ series: [SPEC, SPEC2] });
        show({ idx: 0, data: [[T0], [10], [5]] });
        expect(valToPos.mock.calls[0][0]).toBe(10);
        valToPos.mockClear();
        show({ idx: 0, data: [[T0], [null], [5]] });
        expect(valToPos.mock.calls[0][0]).toBe(5);
    });

    it("centres on the cursor, and keeps inside the chart", () => {
        const { tip, show } = tooltip();
        // Half its width is 30; the chart is 200 wide.
        show({ idx: 0, offsetLeft: 40, left: 100 });
        expect(tip.style.left).toBe("140px");
        show({ idx: 0, offsetLeft: 40, left: 190 });
        expect(tip.style.left).toBe("170px");
        show({ idx: 0, offsetLeft: 40, left: -30 });
        expect(tip.style.left).toBe("30px");
        show({ idx: 0, offsetLeft: 40, left: null });
        expect(tip.style.left).toBe("40px");
    });
});

describe("TimeChart: resizing", () => {
    it("watches its canvas, and stops watching with the pane", () => {
        const { unmount, container } = mountChart();
        expect(observers[0].observed).toEqual([container.querySelector(".sysinfo-chart-canvas")]);
        expect(observers[0].disconnected).toBe(0);
        unmount();
        expect(observers[0].disconnected).toBe(1);
        expect(last().destroyed).toBe(true);
    });

    it("lays out at the whole pixels of the newest entry, width first", () => {
        mountChart({}, null);
        layout(600.7, 200.9, 3);
        expect([last().opts.width, last().opts.height]).toEqual([600, 200]);
    });

    it("gives uPlot a new size, not a new chart, but no more than one every 100ms", async () => {
        mountChart();
        layout(500, 180);
        expect(last().sizes).toEqual([]);
        await vi.advanceTimersByTimeAsync(60);
        expect(last().sizes).toEqual([]);
        await vi.advanceTimersByTimeAsync(60);
        expect(last().sizes).toEqual([{ width: 500, height: 180 }]);
        expect(made.charts).toHaveLength(1);
    });

    it("builds nothing for a zero width or a zero height at the start", async () => {
        mountChart({}, [0, 200]);
        expect(made.charts).toHaveLength(0);
        await vi.advanceTimersByTimeAsync(150);
        layout(600, 0);
        expect(made.charts).toHaveLength(0);
        await vi.advanceTimersByTimeAsync(150);
        layout(600, 200);
        expect(made.charts).toHaveLength(1);
    });

    it("hands uPlot a size with no room as it is, and keeps its chart", async () => {
        mountChart();
        await vi.advanceTimersByTimeAsync(150);
        layout(0, 0);
        expect(made.charts).toHaveLength(1);
        expect(made.charts[0].sizes).toEqual([{ width: 0, height: 0 }]);
        expect(made.charts[0].destroyed).toBe(false);
    });
});

describe("TimeChart: a new chart or new data", () => {
    it("is built again, and the old one destroyed, when its series change", () => {
        const { set } = mountChart();
        const first = last();
        set.setSeries([SPEC, SPEC2]);
        expect(made.charts).toHaveLength(2);
        expect(first.destroyed).toBe(true);
        expect(last().opts.series).toHaveLength(3);
    });

    it("is built again when its time labels, its y labels or its zoom change", () => {
        const { set } = mountChart();
        set.setTimeAxis(false);
        expect(made.charts).toHaveLength(2);
        expect(last().opts.axes[0].size).toBe(4);
        set.setYLabels(false);
        expect(made.charts).toHaveLength(3);
        set.setZoom(1.5);
        expect(made.charts).toHaveLength(4);
        expect(last().opts.axes[0].font).toMatch(/^18px /);
    });

    it("is built again when it goes bare, or back", async () => {
        mountChart();
        await vi.advanceTimersByTimeAsync(150);
        layout(600, 30);
        await vi.advanceTimersByTimeAsync(150);
        expect(made.charts).toHaveLength(2);
        expect(made.charts[0].destroyed).toBe(true);
        expect(last().opts.axes).toEqual([{ show: false }, { show: false }]);
        layout(600, 300);
        await vi.advanceTimersByTimeAsync(150);
        expect(made.charts).toHaveLength(3);
        expect(last().opts.axes[0].show).not.toBe(false);
    });

    it("takes new rows with setData, in the series' data, not a new chart", () => {
        const { set } = mountChart();
        set.setPlotData(rowsOf(sample(BASE, 40), sample(BASE + 1000, 42), sample(BASE + 2000, 44)));
        expect(made.charts).toHaveLength(1);
        expect(last().setDataCalls).toHaveLength(1);
        expect(last().setDataCalls[0][1]).toEqual([40, 42, 44]);
        expect(last().opts.scales.x.range()).toEqual([(BASE + 2000) / 1000 - 300, (BASE + 2000) / 1000]);
    });

    it("takes new data when the window's length or its interval changes", () => {
        const { set } = mountChart();
        set.setTargetLen(100);
        expect(last().setDataCalls).toHaveLength(1);
        expect(last().opts.scales.x.range()).toEqual([(BASE + 1000) / 1000 - 100, (BASE + 1000) / 1000]);
        set.setIntervalSecs(2);
        expect(last().setDataCalls).toHaveLength(2);
        expect(last().opts.scales.x.range()).toEqual([(BASE + 1000) / 1000 - 200, (BASE + 1000) / 1000]);
        expect(made.charts).toHaveLength(1);
    });
});

describe("TimeChart: its markup", () => {
    it("is a plain chart box with a canvas, a hidden tip, and no title unless it has one", () => {
        const { root } = mountChart();
        expect(root.className.split(/\s+/).filter(Boolean)).toEqual(["sysinfo-chart"]);
        expect(root.querySelector(".sysinfo-chart-title")).toBeNull();
        expect(root.querySelector(".sysinfo-chart-canvas")).not.toBeNull();
        expect(root.querySelector<HTMLElement>(".sysinfo-chart-tip")!.style.display).toBe("none");
    });

    it("marks a top-row chart, and adds the class it was given", () => {
        const { root } = mountChart({ top: true, class: "mine" });
        expect(root).toHaveClass("sysinfo-chart", "sysinfo-chart--top", "mine");
        cleanup();
        const plain = mountChart({ class: "mine" });
        expect(plain.root).not.toHaveClass("sysinfo-chart--top");
        expect(plain.root).toHaveClass("mine");
    });

    it("shows the title it was given over the plot", () => {
        const { root } = mountChart({ title: "CPU: 40%" });
        expect(root.querySelector(".sysinfo-chart-title")).toHaveTextContent(/^CPU: 40%$/);
    });
});

describe("SingleLinePlot", () => {
    const META = { name: "CPU use", label: "%", color: "#112233", decimalPlaces: 0, maxy: 50 } as TimeSeriesMeta;

    function plot(over: Partial<Parameters<typeof SingleLinePlot>[0]> = {}) {
        // One plain object, so a prop given as undefined stays undefined (merged props would skip it).
        const props = {
            plotData: ROWS,
            yval: "cpu",
            yvalMeta: META,
            blockId: "b",
            defaultColor: "#a3e635",
            targetLen: 300,
            intervalSecs: 1,
            ...over,
        };
        const result = render(() => <SingleLinePlot {...props} />);
        layout(600, 200);
        return { ...result, root: result.container.querySelector(".sysinfo-chart") as HTMLElement };
    }

    it("titles itself with its name and its figures, in the metric's unit", () => {
        const { root } = plot();
        const format = (v: number) => unitFormat(META)(v);
        expect(root.querySelector(".sysinfo-chart-title")!.textContent).toBe(
            titled("CPU use", figures(ROWS, "cpu", format))
        );
        expect(root.querySelector(".sysinfo-chart-title")!.textContent).toContain("%");
    });

    it("names itself by its key and colours itself by default when it has no metadata", () => {
        const { root } = plot({ yvalMeta: undefined as unknown as TimeSeriesMeta });
        expect(root.querySelector(".sysinfo-chart-title")!.textContent).toMatch(/^cpu/);
        expect(last().opts.series[1]).toMatchObject({ label: "cpu", stroke: "R(#a3e635)" });
    });

    it("takes its colour and its label from its metadata, and its data from its key", () => {
        plot();
        expect(last().opts.series[1]).toMatchObject({ label: "CPU use", stroke: "R(#112233)" });
        expect(last().data[1]).toEqual([40, 42]);
    });

    it("takes its range from its metadata", () => {
        plot();
        expect(last().opts.scales.y.range()).toEqual([0, 50]);
    });

    it("has no title as a sparkline, and a thin line", () => {
        const { root } = plot({ sparkline: true });
        expect(root.querySelector(".sysinfo-chart-title")).toBeNull();
        expect(last().opts.series[1].width).toBe(1.5);
    });

    it("passes down its window, its crosshair key, its layout and its class", () => {
        const { root } = plot({
            targetLen: 100,
            intervalSecs: 2,
            syncKey: "sk",
            timeAxis: false,
            yLabels: false,
            top: true,
        });
        const { opts } = last();
        expect(opts.scales.x.range()).toEqual([(BASE + 1000) / 1000 - 200, (BASE + 1000) / 1000]);
        expect(opts.cursor.sync.key).toBe("sk");
        expect(opts.axes[0].size).toBe(4);
        opts.axes[1].values(null, [0, 50]);
        opts.hooks.draw[0](fakeU({ min: 0, max: 100 }));
        expect(root.querySelectorAll(".sysinfo-chart-ylabel")).toHaveLength(0);
        expect(root).toHaveClass("sysinfo-chart--top", "sysinfo-grid-chart");
    });

    it("draws its y labels unless told not to", () => {
        const { root } = plot();
        last().opts.axes[1].values(null, [0, 50]);
        last().opts.hooks.draw[0](fakeU({ min: 0, max: 100 }));
        expect(root.querySelectorAll(".sysinfo-chart-ylabel")).toHaveLength(2);
    });
});
