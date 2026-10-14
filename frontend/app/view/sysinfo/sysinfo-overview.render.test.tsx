// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Overview as rendered: its tiles' figures, the window and pause controls,
 * the table instead of the charts, and an in/out chart drawing out below the axis.
 * docs/reports/REPORT_SYSINFO_REWRITE_AND_MODERN_MONITOR_2026_10_13.md section 5.3.
 */
import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
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
});
const DATA = [sample(0, 20), sample(1, 60), sample(2, 40)];

function mount(over: Partial<Parameters<typeof Overview>[0]> = {}) {
    const props = {
        plotData: DATA,
        targetLen: 121,
        intervalSecs: 1,
        windowPoints: 120,
        onWindow: vi.fn(),
        paused: false,
        onPause: vi.fn(),
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
});
