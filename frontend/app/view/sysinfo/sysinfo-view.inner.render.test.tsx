// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the Sysinfo view shows once it is connected (`SysinfoViewInner`), beyond
 * sysinfo-view.render.test.tsx: the charts taking the model's samples on a timer, the grid's
 * columns and what each chart is told, the control row's wiring, the table, and the pane's zoom.
 */
import { cleanup, render } from "@solidjs/testing-library";
import type { ComponentProps } from "solid-js";
import { createSignal, useContext } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { SysinfoBarProps } from "./sysinfo-bar";
import type { SysinfoViewModel } from "./sysinfo-model";
import type { MeasureTable, Overview } from "./sysinfo-overview";
import type { SingleLinePlot } from "./sysinfo-plot-lazy";
import type { DataItem } from "./sysinfo-types";
import { SysinfoZoomContext } from "./sysinfo-zoom";

type PlotProps = ComponentProps<typeof SingleLinePlot>;
type OverviewProps = ComponentProps<typeof Overview>;
type TableProps = ComponentProps<typeof MeasureTable>;

const seen = vi.hoisted(() => ({
    bar: [] as SysinfoBarProps[],
    overview: [] as OverviewProps[],
    table: [] as TableProps[],
    plots: [] as PlotProps[],
    zoom: [] as Array<() => number>,
    resizing: false,
}));

vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));
vi.mock("@/app/store/pane-content-holds", () => ({
    holdPaneContent: () => () => {},
    trackPaneContent: () => () => {},
}));
vi.mock("@/app/platform/window-resize", () => ({ windowResizing: () => seen.resizing }));
vi.mock("./sysinfo-plot-lazy", () => ({
    SingleLinePlot: (props: PlotProps) => {
        seen.plots.push(props);
        return null;
    },
}));
vi.mock("./sysinfo-overview", () => ({
    Overview: (props: OverviewProps) => {
        seen.overview.push(props);
        return null;
    },
    MeasureTable: (props: TableProps) => {
        seen.table.push(props);
        return null;
    },
}));
vi.mock("./sysinfo-bar", () => ({
    SysinfoBar: (props: SysinfoBarProps) => {
        seen.bar.push(props);
        seen.zoom.push(useContext(SysinfoZoomContext));
        return null;
    },
}));

import { SysinfoView } from "./sysinfo-view";

beforeEach(() => {
    vi.useFakeTimers();
});

afterEach(() => {
    cleanup();
    vi.useRealTimers();
    seen.bar.length = 0;
    seen.overview.length = 0;
    seen.table.length = 0;
    seen.plots.length = 0;
    seen.zoom.length = 0;
    seen.resizing = false;
});

const row = (ts: number): DataItem => ({ ts, cpu: ts });

type Setup = {
    plotType?: string;
    metrics?: string[];
    data?: DataItem[];
    loading?: boolean;
    visible?: boolean;
    meta?: Record<string, unknown>;
    numPoints?: number;
    intervalSecs?: number;
    windowSecs?: number;
    focus?: string;
    controls?: boolean;
    plotMeta?: Map<string, TimeSeriesMeta>;
};

function mount(setup: Setup = {}) {
    const [plotType, setPlotType] = createSignal(setup.plotType ?? "Overview");
    const [metrics, setMetrics] = createSignal(setup.metrics ?? ["cpu"]);
    const [data, setData] = createSignal(setup.data ?? [row(1)]);
    const [loading, setLoading] = createSignal(setup.loading ?? false);
    const [visible, setVisible] = createSignal(setup.visible ?? true);
    const [meta, setMeta] = createSignal(setup.meta ?? {});
    const [focus, setFocus] = createSignal(setup.focus ?? "cpu");
    const fns = {
        setWindow: vi.fn(),
        setFocus: vi.fn(),
        setSimpleView: vi.fn(),
        setControlsShown: vi.fn(),
        dispose: vi.fn(),
        loadInitialData: vi.fn(),
        appendData: vi.fn(),
    };
    const model = {
        blockId: "model-block",
        meta,
        plotTypeSelectedAtom: plotType,
        dataAtom: data,
        loadingAtom: loading,
        connStatus: () => ({ status: "connected" }),
        connection: () => "local",
        metrics,
        plotMetaAtom: () => setup.plotMeta ?? new Map<string, TimeSeriesMeta>(),
        numPoints: () => setup.numPoints ?? 120,
        intervalSecsAtom: () => setup.intervalSecs ?? 1,
        windowSecs: () => setup.windowSecs ?? 120,
        focus,
        visible,
        controlsShown: () => setup.controls ?? false,
        ...fns,
    } as unknown as SysinfoViewModel;
    const r = render(() => <SysinfoView blockId="prop-block" model={model} />);
    return { ...r, model, fns, setPlotType, setMetrics, setData, setLoading, setVisible, setMeta, setFocus };
}

const bar = () => seen.bar[seen.bar.length - 1];
const overview = () => seen.overview[seen.overview.length - 1];
const table = () => seen.table[seen.table.length - 1];
const grid = (c: HTMLElement) => c.querySelector(".sysinfo-grid");

describe("the charts take the model's samples on a timer", () => {
    it("starts with the samples the model has when the pane opens", () => {
        const data = [row(1), row(2)];
        mount({ data });
        expect(overview().plotData).toBe(data);
    });

    it("starts with them even while a load is in flight", () => {
        const data = [row(1), row(2)];
        mount({ data, loading: true });
        expect(overview().plotData).toBe(data);
    });

    it("keeps them until a whole second has passed, however many samples arrive", () => {
        const first = [row(1)];
        const { setData } = mount({ data: first });
        setData([row(1), row(2)]);
        setData([row(1), row(2), row(3)]);
        vi.advanceTimersByTime(999);
        expect(overview().plotData).toBe(first);
    });

    it("takes the model's samples at the second", () => {
        const { setData } = mount();
        const next = [row(1), row(2)];
        setData(next);
        vi.advanceTimersByTime(1000);
        expect(overview().plotData).toBe(next);
    });

    it("takes them again at every second after", () => {
        const { setData } = mount();
        setData([row(1), row(2)]);
        vi.advanceTimersByTime(1000);
        const later = [row(1), row(2), row(3)];
        setData(later);
        vi.advanceTimersByTime(1000);
        expect(overview().plotData).toBe(later);
    });

    it("passes the same samples to the grid's charts", () => {
        const { setData } = mount({ plotType: "CPU", metrics: ["cpu", "mem"] });
        const next = [row(1), row(2)];
        setData(next);
        vi.advanceTimersByTime(1000);
        expect(seen.plots.map((p) => p.plotData)).toEqual([next, next]);
    });

    it("takes none while the pane is paused", () => {
        const first = [row(1)];
        const { setData } = mount({ data: first });
        bar().onPause(true);
        setData([row(1), row(2)]);
        vi.advanceTimersByTime(3000);
        expect(overview().plotData).toBe(first);
    });

    it("pausing does not change what the charts show", () => {
        const first = [row(1)];
        const { setData } = mount({ data: first });
        setData([row(1), row(2)]);
        bar().onPause(true);
        expect(overview().plotData).toBe(first);
    });

    it("takes the samples at once when the pause is lifted", () => {
        const { setData } = mount();
        bar().onPause(true);
        const next = [row(1), row(2)];
        setData(next);
        vi.advanceTimersByTime(3000);
        bar().onPause(false);
        expect(overview().plotData).toBe(next);
        expect(bar().paused).toBe(false);
    });

    it("takes none while the window is being resized, and catches up at the next second", () => {
        const first = [row(1)];
        const { setData } = mount({ data: first });
        const next = [row(1), row(2)];
        setData(next);
        seen.resizing = true;
        vi.advanceTimersByTime(2000);
        expect(overview().plotData).toBe(first);
        seen.resizing = false;
        vi.advanceTimersByTime(1000);
        expect(overview().plotData).toBe(next);
    });

    it("takes none while the pane is off screen", () => {
        const first = [row(1)];
        const { setData, setVisible } = mount({ data: first });
        setVisible(false);
        setData([row(1), row(2)]);
        vi.advanceTimersByTime(3000);
        expect(overview().plotData).toBe(first);
    });

    it("stops the timer with the pane", () => {
        const before = vi.getTimerCount();
        const { unmount } = mount();
        expect(vi.getTimerCount()).toBeGreaterThan(before);
        unmount();
        expect(vi.getTimerCount()).toBe(before);
    });
});

describe("a load finishing", () => {
    it("shows the samples at once, not at the next second", () => {
        const { setData, setLoading } = mount({ loading: true, data: [] });
        const history = [row(1), row(2), row(3)];
        setData(history);
        setLoading(false);
        expect(overview().plotData).toBe(history);
    });

    it("does the same after a reload", () => {
        const { setData, setLoading } = mount();
        setLoading(true);
        const history = [row(5), row(6)];
        setData(history);
        setLoading(false);
        expect(overview().plotData).toBe(history);
    });

    it("leaves the charts alone when a load starts", () => {
        const first = [row(1)];
        const { setData, setLoading } = mount({ data: first });
        setData([row(1), row(2)]);
        setLoading(true);
        expect(overview().plotData).toBe(first);
    });

    it("does not take the samples on every sample once it is done", () => {
        const { setData, setLoading } = mount({ loading: true });
        setLoading(false);
        const shown = overview().plotData;
        setData([row(1), row(2)]);
        expect(overview().plotData).toBe(shown);
    });
});

describe("what the control row is told and does", () => {
    it("gets the window, the interval and whether the controls are shown", () => {
        mount({ windowSecs: 300, intervalSecs: 5, controls: true });
        expect(bar().windowSecs).toBe(300);
        expect(bar().intervalSecs).toBe(5);
        expect(bar().controls).toBe(true);
    });

    it("has the controls hidden when the model says so", () => {
        mount({ controls: false });
        expect(bar().controls).toBe(false);
    });

    it("sets the window, and shows or hides the controls, on the model", () => {
        const { fns } = mount();
        bar().onWindow(600);
        expect(fns.setWindow).toHaveBeenCalledWith(600);
        bar().onControls(true);
        expect(fns.setControlsShown).toHaveBeenCalledWith(true);
        bar().onControls(false);
        expect(fns.setControlsShown).toHaveBeenLastCalledWith(false);
    });

    it("starts running, not as a table", () => {
        mount();
        expect(bar().paused).toBe(false);
        expect(bar().table).toBe(false);
    });

    it("shows a pause, and its end", () => {
        mount();
        bar().onPause(true);
        expect(bar().paused).toBe(true);
        bar().onPause(false);
        expect(bar().paused).toBe(false);
    });

    it("offers the Advanced areas in the Advanced mode, with the area the model shows", () => {
        mount({ plotType: "Overview", focus: "mem" });
        expect(bar().views).toEqual([
            { value: "cpu", label: "CPU" },
            { value: "mem", label: "Memory" },
            { value: "disk", label: "Disk" },
            { value: "net", label: "Network" },
            { value: "everything", label: "Everything" },
        ]);
        expect(bar().view).toBe("mem");
    });

    it("offers the simple views otherwise, by their short names, with the plot type the model shows", () => {
        mount({ plotType: "Net" });
        expect(bar().views).toEqual([
            { value: "CPU", label: "CPU" },
            { value: "All CPU", label: "Cores" },
            { value: "Mem", label: "Memory" },
            { value: "Net", label: "Network" },
            { value: "Net (Sent/Recv)", label: "Net in/out" },
            { value: "Disk I/O", label: "Disk" },
            { value: "Disk I/O (R/W)", label: "Disk r/w" },
            { value: "CPU + Mem", label: "CPU+Mem" },
            { value: "CPU + Mem + Net", label: "CPU+Mem+Net" },
        ]);
        expect(bar().view).toBe("Net");
    });

    it("changes the area in the Advanced mode, not the simple view", () => {
        const { fns } = mount({ plotType: "Overview" });
        bar().onView("disk");
        expect(fns.setFocus).toHaveBeenCalledWith("disk");
        expect(fns.setSimpleView).not.toHaveBeenCalled();
    });

    it("changes the simple view in a simple mode, not the area", () => {
        const { fns } = mount({ plotType: "CPU" });
        bar().onView("Mem");
        expect(fns.setSimpleView).toHaveBeenCalledWith("Mem");
        expect(fns.setFocus).not.toHaveBeenCalled();
    });

    it("follows the model from one mode to the other", () => {
        const { setPlotType, setFocus } = mount({ plotType: "Overview", focus: "net" });
        expect(bar().view).toBe("net");
        setPlotType("Mem");
        expect(bar().view).toBe("Mem");
        setPlotType("Overview");
        setFocus("everything");
        expect(bar().view).toBe("everything");
    });
});

describe("the Advanced mode", () => {
    it("shows the overview, told the samples, the area, the length and the interval", () => {
        const data = [row(1), row(2)];
        const { container } = mount({ plotType: "Overview", data, focus: "net", numPoints: 119, intervalSecs: 2 });
        expect(overview().plotData).toBe(data);
        expect(overview().focus).toBe("net");
        expect(overview().targetLen).toBe(120);
        expect(overview().intervalSecs).toBe(2);
        expect(overview().syncKey).toBe("sysinfo-model-block");
        expect(grid(container)).toBeNull();
        expect(seen.plots).toHaveLength(0);
    });

    it("follows the area the model shows", () => {
        const { setFocus } = mount({ plotType: "Overview", focus: "cpu" });
        setFocus("disk");
        expect(overview().focus).toBe("disk");
    });

    it("is left for the grid by a simple plot type, and drawn again on return", () => {
        const { container, setPlotType } = mount({ plotType: "Overview" });
        setPlotType("CPU");
        expect(grid(container)).not.toBeNull();
        const drawn = seen.overview.length;
        setPlotType("Overview");
        expect(grid(container)).toBeNull();
        expect(seen.overview.length).toBeGreaterThan(drawn);
    });
});

describe("the grid of simple charts", () => {
    it("is a scrolling column holding the grid, with one chart for each metric", () => {
        const { container } = mount({ plotType: "CPU", metrics: ["cpu", "mem", "net"] });
        const g = grid(container);
        expect(g?.parentElement?.classList.contains("overflow-y-auto")).toBe(true);
        expect(g?.parentElement?.classList.contains("flex-grow")).toBe(true);
        expect(seen.plots.map((p) => p.yval)).toEqual(["cpu", "mem", "net"]);
        expect(seen.overview).toHaveLength(0);
    });

    it("tells each chart its metric's meta, the block, the colour, the length, the interval and the sync key", () => {
        const memMeta = { name: "Memory", label: "%", color: "red" } as TimeSeriesMeta;
        const cpuMeta = { name: "CPU", label: "%", color: "blue" } as TimeSeriesMeta;
        const data = [row(1), row(2)];
        mount({
            plotType: "CPU + Mem",
            metrics: ["cpu", "mem"],
            data,
            numPoints: 59,
            intervalSecs: 3,
            plotMeta: new Map([
                ["cpu", cpuMeta],
                ["mem", memMeta],
            ]),
        });
        expect(seen.plots).toHaveLength(2);
        const [a, b] = seen.plots;
        expect(a.yvalMeta).toBe(cpuMeta);
        expect(b.yvalMeta).toBe(memMeta);
        for (const p of seen.plots) {
            expect(p.plotData).toBe(data);
            expect(p.blockId).toBe("model-block");
            expect(p.defaultColor).toBe("var(--accent-color)");
            expect(p.targetLen).toBe(60);
            expect(p.intervalSecs).toBe(3);
            expect(p.syncKey).toBe("sysinfo-model-block");
        }
    });

    it("draws a chart for a metric that has no meta, without one", () => {
        mount({ plotType: "CPU", metrics: ["cpu:7"] });
        expect(seen.plots[0].yvalMeta).toBeUndefined();
    });

    it("puts one metric in one column", () => {
        const { container } = mount({ plotType: "CPU", metrics: ["cpu"] });
        expect(grid(container)?.classList.contains("grid-cols-1")).toBe(false);
        expect(grid(container)?.classList.contains("grid-cols-2")).toBe(false);
        const p = seen.plots[0];
        expect([p.timeAxis, p.yLabels, p.top]).toEqual([true, true, true]);
    });

    it("puts two metrics in a grid with no column count set", () => {
        const { container } = mount({ plotType: "CPU + Mem", metrics: ["cpu", "mem"] });
        expect(grid(container)?.classList.contains("grid-cols-1")).toBe(false);
        expect(grid(container)?.classList.contains("grid-cols-2")).toBe(false);
        // One column: the bottom chart has the time axis, the top one is under the buttons.
        expect(seen.plots.map((p) => p.timeAxis)).toEqual([false, true]);
        expect(seen.plots.map((p) => p.top)).toEqual([true, false]);
    });

    it("puts the cores of a three-core machine in two columns", () => {
        const { container } = mount({ plotType: "All CPU", metrics: ["cpu:0", "cpu:1", "cpu:2"] });
        expect(grid(container)?.classList.contains("grid-cols-2")).toBe(true);
        expect(grid(container)?.classList.contains("grid-cols-1")).toBe(false);
    });

    it("keeps CPU + Mem + Net in one full-width column", () => {
        const { container } = mount({ plotType: "CPU + Mem + Net", metrics: ["cpu", "mem", "net"] });
        expect(grid(container)?.classList.contains("grid-cols-1")).toBe(true);
        expect(grid(container)?.classList.contains("grid-cols-2")).toBe(false);
        expect(seen.plots.map((p) => p.timeAxis)).toEqual([false, false, true]);
        expect(seen.plots.map((p) => p.top)).toEqual([true, false, false]);
        expect(seen.plots.map((p) => p.yLabels)).toEqual([true, true, true]);
    });

    it("keeps CPU + Mem + Net in one column with any number of metrics", () => {
        const { container } = mount({ plotType: "CPU + Mem + Net", metrics: ["cpu", "mem", "net", "disk", "x"] });
        expect(grid(container)?.classList.contains("grid-cols-1")).toBe(true);
        expect(grid(container)?.classList.contains("grid-cols-2")).toBe(false);
        expect(seen.plots.map((p) => p.top)).toEqual([true, false, false, false, false]);
    });

    it("gives both cores of a two-core machine their y labels: they share one column", () => {
        mount({ plotType: "All CPU", metrics: ["cpu:0", "cpu:1"] });
        expect(seen.plots.map((p) => p.yLabels)).toEqual([true, true]);
    });

    it("places the labels of four cores in two columns by row and column", () => {
        mount({ plotType: "All CPU", metrics: ["cpu:0", "cpu:1", "cpu:2", "cpu:3"] });
        // Every chart is 0-100%, so only the left column writes the y labels.
        expect(seen.plots.map((p) => p.yLabels)).toEqual([true, false, true, false]);
        expect(seen.plots.map((p) => p.timeAxis)).toEqual([false, false, true, true]);
        expect(seen.plots.map((p) => p.top)).toEqual([true, true, false, false]);
    });

    it("gives every chart its y labels when the ranges differ", () => {
        mount({ plotType: "Mem", metrics: ["cpu", "mem", "net", "disk"] });
        expect(seen.plots.map((p) => p.yLabels)).toEqual([true, true, true, true]);
    });

    it("treats cpu and the cores as one range, and anything else as a different one", () => {
        mount({ plotType: "All CPU", metrics: ["cpu", "cpu:0", "mem", "cpu:1"] });
        expect(seen.plots.map((p) => p.yLabels)).toEqual([true, true, true, true]);
    });

    it("moves to two columns and back as the metrics change", () => {
        const { container, setMetrics } = mount({ plotType: "All CPU", metrics: ["cpu:0", "cpu:1"] });
        expect(grid(container)?.classList.contains("grid-cols-2")).toBe(false);
        setMetrics(["cpu:0", "cpu:1", "cpu:2", "cpu:3"]);
        expect(grid(container)?.classList.contains("grid-cols-2")).toBe(true);
        setMetrics(["cpu:0"]);
        expect(grid(container)?.classList.contains("grid-cols-2")).toBe(false);
    });

    it("follows the plot type into and out of the single column", () => {
        const { container, setPlotType } = mount({ plotType: "CPU + Mem", metrics: ["cpu", "mem", "net"] });
        expect(grid(container)?.classList.contains("grid-cols-2")).toBe(true);
        setPlotType("CPU + Mem + Net");
        expect(grid(container)?.classList.contains("grid-cols-1")).toBe(true);
        expect(grid(container)?.classList.contains("grid-cols-2")).toBe(false);
    });

    it("draws a chart for each metric as the list grows", () => {
        const { setMetrics } = mount({ plotType: "CPU", metrics: ["cpu"] });
        expect(seen.plots.map((p) => p.yval)).toEqual(["cpu"]);
        setMetrics(["cpu", "mem"]);
        expect(seen.plots.map((p) => p.yval)).toContain("mem");
    });
});

describe("the table", () => {
    it("takes the place of the charts, in the scroll box, and the charts return when it is closed", () => {
        const { container } = mount({ plotType: "Overview" });
        bar().onTable(true);
        expect(bar().table).toBe(true);
        expect(container.querySelector(".sysinfo-table-scroll")).not.toBeNull();
        expect(table()).toBeDefined();
        bar().onTable(false);
        expect(bar().table).toBe(false);
        expect(container.querySelector(".sysinfo-table-scroll")).toBeNull();
    });

    it("replaces the grid too", () => {
        const { container } = mount({ plotType: "CPU", metrics: ["cpu", "mem"] });
        bar().onTable(true);
        expect(grid(container)).toBeNull();
        bar().onTable(false);
        expect(grid(container)).not.toBeNull();
    });

    it("lists a simple view's measures, with the samples", () => {
        const data = [row(1), row(2)];
        mount({ plotType: "CPU + Mem", metrics: ["cpu", "mem"], data });
        bar().onTable(true);
        expect(table().keys).toEqual(["cpu", "mem"]);
        expect(table().focus).toBeUndefined();
        expect(table().plotData).toBe(data);
    });

    it("lists an Advanced view's area, with the samples", () => {
        const data = [row(1), row(2)];
        mount({ plotType: "Overview", focus: "disk", data });
        bar().onTable(true);
        expect(table().focus).toBe("disk");
        expect(table().keys).toBeUndefined();
        expect(table().plotData).toBe(data);
    });

    it("stays open from one mode to the other", () => {
        const { container, setPlotType } = mount({ plotType: "Overview" });
        bar().onTable(true);
        setPlotType("Mem");
        expect(bar().table).toBe(true);
        expect(container.querySelector(".sysinfo-table-scroll")).not.toBeNull();
        expect(table().keys).toEqual(["cpu"]);
    });

    it("shows the samples that come at the timer", () => {
        const { setData } = mount({ plotType: "CPU" });
        bar().onTable(true);
        const next = [row(1), row(2)];
        setData(next);
        vi.advanceTimersByTime(1000);
        expect(table().plotData).toBe(next);
    });
});

describe("the width the buttons take", () => {
    const widthOf = (c: HTMLElement) => c.querySelector(".sysinfo-pane")?.getAttribute("style") ?? "";

    it("is none until the row says", () => {
        const { container } = mount();
        expect(widthOf(container)).toContain("--sysinfo-actions-w: 0px");
    });

    it("is passed to the pane as a style variable", () => {
        const { container } = mount();
        bar().onActionsWidth?.(87);
        expect(widthOf(container)).toContain("--sysinfo-actions-w: 87px");
        bar().onActionsWidth?.(40);
        expect(widthOf(container)).toContain("--sysinfo-actions-w: 40px");
    });
});

describe("the pane's zoom", () => {
    it("is 1 when the pane has none", () => {
        mount();
        expect(seen.zoom[0]()).toBe(1);
    });

    it("reaches the control row from the pane's meta", () => {
        mount({ meta: { "term:zoom": 1.5 } });
        expect(seen.zoom[0]()).toBe(1.5);
    });

    it("follows the meta", () => {
        const { setMeta } = mount({ meta: { "term:zoom": 1.5 } });
        setMeta({ "term:zoom": 0.75 });
        expect(seen.zoom[0]()).toBe(0.75);
        setMeta({});
        expect(seen.zoom[0]()).toBe(1);
    });
});
