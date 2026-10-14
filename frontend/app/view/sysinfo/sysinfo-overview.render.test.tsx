// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Overview as rendered: its tiles' figures, the window and pause controls,
 * the table instead of the charts, an in/out chart drawing out below the axis,
 * and each area shown alone in at most three charts.
 * docs/reports/REPORT_SYSINFO_REWRITE_AND_MODERN_MONITOR_2026_10_13.md section 5.3.
 */
import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const made = vi.hoisted(() => ({ charts: [] as any[] }));

vi.mock("uplot", () => {
    class FakeUPlot {
        opts: any;
        data: any;
        constructor(opts: any, data: any, el: HTMLElement) {
            this.opts = opts;
            this.data = data;
            el.appendChild(document.createElement("canvas"));
            made.charts.push(this);
        }
        setData(d: any) {
            this.data = d;
        }
        setSize() {}
        destroy() {}
    }
    return { default: FakeUPlot };
});
vi.mock("uplot/dist/uPlot.min.css", () => ({}));

import { Overview } from "./sysinfo-overview";
import type { DataItem } from "./sysinfo-types";

class FixedSizeObserver {
    constructor(private cb: ResizeObserverCallback) {}
    observe(el: Element) {
        this.cb([{ target: el, contentRect: { width: 400, height: 120 } } as ResizeObserverEntry], this as any);
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

const base = 1_791_900_000_000;
const sample = (i: number, cpu: number): DataItem => ({
    ts: base + i * 1000,
    cpu,
    "cpu:0": cpu,
    "cpu:1": 100 - cpu,
    "mem:used": 12,
    "mem:total": 32,
    "net:bytesrecv": 2.5,
    "net:bytessent": 0.4,
    "disk:read": 0.05,
    "disk:write": 1.2,
    "mem:available": 20,
    "disk:vol:/data:free_gb": 100,
    "disk:vol:/data:total_gb": 500,
});
const DATA = [sample(0, 20), sample(1, 60), sample(2, 40)];

function mount(over: Partial<Parameters<typeof Overview>[0]> = {}) {
    const props = {
        plotData: DATA,
        targetLen: 121,
        intervalSecs: 1,
        windowSecs: 120,
        onWindow: vi.fn(),
        paused: false,
        onPause: vi.fn(),
        focus: null,
        onFocus: vi.fn(),
        syncKey: "sysinfo-b",
        ...over,
    };
    render(() => <Overview {...props} />);
    return props;
}

const tileValue = (name: string) =>
    [...document.querySelectorAll(".sysinfo-tile")]
        .find((t) => t.querySelector(".sysinfo-tile-name")?.textContent === name)
        ?.querySelector(".sysinfo-tile-value")?.textContent;

describe("Overview", () => {
    it("shows each area's figure now, with its low and high over the window", async () => {
        mount();
        await vi.advanceTimersByTimeAsync(200);
        expect(tileValue("CPU")).toBe("40%");
        expect(tileValue("Memory")).toBe("12.0 GB of 32.0 GB");
        expect(tileValue("Network")).toBe("↓ 2.5 MB/s  ↑ 400 KB/s");
        expect(tileValue("Disk")).toBe("R 50 KB/s  W 1.2 MB/s");
        const cpuTile = [...document.querySelectorAll(".sysinfo-tile")][0];
        expect(cpuTile.querySelector(".sysinfo-tile-range")?.textContent).toBe("20% – 60%");
        // In and out each give their peak.
        const range = (name: string) =>
            [...document.querySelectorAll(".sysinfo-tile")]
                .find((t) => t.querySelector(".sysinfo-tile-name")?.textContent === name)
                ?.querySelector(".sysinfo-tile-range")?.textContent;
        expect(range("Network")).toBe("peak ↓ 2.5 MB/s  ↑ 400 KB/s");
        expect(range("Disk")).toBe("peak R 50 KB/s  W 1.2 MB/s");
    });

    it("draws network out below the axis, on the same chart as in", async () => {
        mount();
        await vi.advanceTimersByTimeAsync(200);
        const net = made.charts.find((c) => c.opts.series.length === 3 && c.opts.series[1].label === "In");
        expect(net).toBeDefined();
        expect(net.data[1]).toEqual([2.5, 2.5, 2.5]);
        expect(net.data[2]).toEqual([-0.4, -0.4, -0.4]);
        // Its axis reads amounts, not negatives.
        expect(net.opts.axes[1].values(null, [-1, 0, 1])).toEqual(["1.0 MB/s", "0 KB/s", "1.0 MB/s"]);
    });

    it("changes the window and pauses through its controls", async () => {
        const props = mount();
        fireEvent.click(screen.getByRole("radio", { name: "15m" }));
        expect(props.onWindow).toHaveBeenCalledWith(900);
        fireEvent.click(screen.getByRole("button", { name: "Pause" }));
        expect(props.onPause).toHaveBeenCalledWith(true);
    });

    it("says it is paused, and offers to resume", () => {
        mount({ paused: true });
        expect(screen.getByText("Paused")).toBeTruthy();
        expect(screen.getByRole("button", { name: "Resume" })).toBeTruthy();
    });

    it("can show the figures as a table instead, every core included", () => {
        mount();
        fireEvent.click(screen.getByRole("button", { name: "Show as a table" }));
        const rows = [...document.querySelectorAll(".sysinfo-overview-table tbody tr")].map((r) =>
            [...r.children].map((c) => c.textContent)
        );
        expect(rows[0]).toEqual(["CPU", "40%", "20%", "60%", "40%"]);
        expect(rows.map((r) => r[0])).toEqual([
            "CPU",
            "Memory used",
            "Network in",
            "Network out",
            "Disk read",
            "Disk write",
            "Core 0",
            "Core 1",
        ]);
        expect(document.querySelector(".sysinfo-overview-charts")).toBeNull();
    });

    it("names its core heatmap with the number of cores", () => {
        mount();
        expect(screen.getByText("Cores (2)")).toBeTruthy();
    });

    it("keeps its tiles and their sparklines across new samples, giving them new data", async () => {
        const [data, setData] = createSignal(DATA.slice(0, 2));
        render(() => (
            <Overview
                plotData={data()}
                targetLen={121}
                intervalSecs={1}
                windowSecs={120}
                onWindow={vi.fn()}
                paused={false}
                onPause={vi.fn()}
                focus={null}
                onFocus={vi.fn()}
                syncKey="sysinfo-b"
            />
        ));
        await vi.advanceTimersByTimeAsync(200);
        const built = made.charts.length;
        const spark = made.charts[0];
        setData(DATA);
        await vi.advanceTimersByTimeAsync(200);
        expect(made.charts.length).toBe(built);
        expect(spark.data[1]).toEqual([20, 60, 40]);
        expect(tileValue("CPU")).toBe("40%");
    });

    it("offers each area alone, and every area again", () => {
        const props = mount();
        fireEvent.click(screen.getByRole("radio", { name: "Network" }));
        expect(props.onFocus).toHaveBeenCalledWith("net");
        fireEvent.click(screen.getByRole("radio", { name: "Overview" }));
        expect(props.onFocus).toHaveBeenLastCalledWith(null);
    });
});

const titles = () => [...document.querySelectorAll(".sysinfo-chart-title")].map((t) => t.textContent);

describe("An area alone", () => {
    it("CPU: no tiles; the CPU, its busiest core against the average, and the cores, titled with their figures", async () => {
        mount({ focus: "cpu" });
        await vi.advanceTimersByTimeAsync(200);
        expect(document.querySelector(".sysinfo-overview-tiles")).toBeNull();
        expect(titles()).toEqual(["CPU: 40% now, 20% – 60%", "Busiest core: 60% now, 60% – 80%", "Cores (2)"]);
        const busiest = made.charts.find((c) => c.opts.series[1].label === "Busiest core");
        // Each sample's higher core: 80, 60, 60.
        expect(busiest.data[1]).toEqual([80, 60, 60]);
    });

    it("network: in and out together, then each on its own scale", async () => {
        mount({ focus: "net" });
        await vi.advanceTimersByTimeAsync(200);
        expect(titles()).toEqual([
            "In above, out below",
            "In: 2.5 MB/s now, 2.5 MB/s – 2.5 MB/s",
            "Out: 400 KB/s now, 400 KB/s – 400 KB/s",
        ]);
        expect(made.charts).toHaveLength(3);
    });

    it("memory: committed only where the machine reports it", async () => {
        mount({ focus: "mem" });
        await vi.advanceTimersByTimeAsync(200);
        expect(titles()).toEqual(["Used: 12.0 GB now, 12.0 GB – 12.0 GB", "Available: 20.0 GB now, 20.0 GB – 20.0 GB"]);
        cleanup();
        made.charts.length = 0;
        const withCommit = DATA.map((d) => ({ ...d, "mem:commit:used": 30, "mem:commit:total": 64 }));
        mount({ focus: "mem", plotData: withCommit });
        await vi.advanceTimersByTimeAsync(200);
        expect(titles()).toHaveLength(3);
        const committed = made.charts.find((c) => c.opts.series[1].label === "Committed");
        // Its scale ends at the commit limit.
        expect(committed.opts.scales.y.range()).toEqual([0, 64]);
    });

    it("disk: three charts and each volume's free space", async () => {
        mount({ focus: "disk" });
        await vi.advanceTimersByTimeAsync(200);
        expect(made.charts).toHaveLength(3);
        expect(screen.getByRole("list", { name: "Volumes" }).textContent).toBe("/data100.0 GB free of 500.0 GB");
    });

    it("lists only its own measures as a table", () => {
        mount({ focus: "net" });
        fireEvent.click(screen.getByRole("button", { name: "Show as a table" }));
        const names = [...document.querySelectorAll(".sysinfo-overview-table tbody th")].map((c) => c.textContent);
        expect(names).toEqual(["Network in", "Network out"]);
    });
});
