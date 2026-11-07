// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Characterisation tests for FocusView: the charts each area (CPU, memory, disk, network) is shown
 * as, in order, with what each chart is handed (series, range, format, axis, title, shared sync
 * key), the titles' figures, the core heatmap, the volume list, and what changes as samples and
 * props change. They pin today's behaviour (written against the unsplit component) so it can be
 * split into named pieces without changing it. The chart and the heatmap are stubs that record the
 * props they were given; the data helpers (figures, ranges, formats, cores, volumes) are real.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

type ChartProps = {
    plotData: DataItem[];
    series: { key: string; label: string; color: string; below?: boolean }[];
    yRange: (rows: DataItem[]) => [number, number];
    format: (v: number) => string;
    targetLen: number;
    intervalSecs: number;
    syncKey: string;
    title: string;
    timeAxis: boolean;
    top?: boolean;
    class: string;
};

type HeatmapProps = {
    plotData: DataItem[];
    cores: string[];
    color: string;
    targetLen: number;
    intervalSecs: number;
};

const h = vi.hoisted(() => ({
    charts: [] as unknown[],
    heatmaps: [] as unknown[],
}));

vi.mock("./sysinfo-plot-lazy", () => ({
    TimeChart: (props: ChartProps) => {
        h.charts.push(props);
        return (
            <div data-testid="chart" class={props.class}>
                {props.title}
            </div>
        );
    },
}));
vi.mock("./sysinfo-heatmap", () => ({
    CoreHeatmap: (props: HeatmapProps) => {
        h.heatmaps.push(props);
        return <div data-testid="heatmap" />;
    },
}));

import { yRange } from "./sysinfo-chart";
import { FocusView } from "./sysinfo-focus";
import { formatGB, formatRate, mirroredRange, withBusiestCore } from "./sysinfo-overview-data";
import { type DataItem, DefaultPlotMeta } from "./sysinfo-types";

const base = 1_791_900_000_000;

function sample(i: number, over: Partial<DataItem> = {}): DataItem {
    return { ts: base + i * 1000, ...over } as DataItem;
}

const CPU_ROWS: DataItem[] = [
    sample(0, { cpu: 20, "cpu:0": 20, "cpu:1": 80, "cpu:10": 5, "cpu:2": 30 }),
    sample(1, { cpu: 60, "cpu:0": 60, "cpu:1": 40, "cpu:10": 5, "cpu:2": 30 }),
    sample(2, { cpu: 40, "cpu:0": 40, "cpu:1": 60, "cpu:10": 5, "cpu:2": 30 }),
];

const MEM_ROWS: DataItem[] = [
    sample(0, { "mem:used": 10, "mem:available": 22, "mem:commit:used": 8, "mem:commit:total": 32 }),
    sample(1, { "mem:used": 14, "mem:available": 18, "mem:commit:used": 9, "mem:commit:total": 32 }),
    sample(2, { "mem:used": 12, "mem:available": 20, "mem:commit:used": 10, "mem:commit:total": 32 }),
];

const DISK_ROWS: DataItem[] = [
    sample(0, { "disk:read": 0.5, "disk:write": 0.1 }),
    sample(1, { "disk:read": 2, "disk:write": 0.2 }),
    sample(2, {
        "disk:read": 1,
        "disk:write": 0.3,
        "disk:vol:/b:free_gb": 50,
        "disk:vol:/b:total_gb": 200,
        "disk:vol:/a:free_gb": 100,
        "disk:vol:/a:total_gb": 500,
    }),
];

const NET_ROWS: DataItem[] = [
    sample(0, { "net:bytesrecv": 3, "net:bytessent": 0.4 }),
    sample(1, { "net:bytesrecv": 1, "net:bytessent": 0.2 }),
    sample(2, { "net:bytesrecv": 2, "net:bytessent": 0.1 }),
];

type Area = "cpu" | "mem" | "disk" | "net";

function mount(
    area: Area,
    plotData: DataItem[],
    extra: { targetLen?: number; intervalSecs?: number; syncKey?: string } = {}
) {
    return render(() => (
        <FocusView
            area={area}
            plotData={plotData}
            targetLen={extra.targetLen ?? 121}
            intervalSecs={extra.intervalSecs ?? 1}
            syncKey={extra.syncKey ?? "sync-1"}
        />
    ));
}

function charts(): ChartProps[] {
    return h.charts as ChartProps[];
}

function heatmaps(): HeatmapProps[] {
    return h.heatmaps as HeatmapProps[];
}

beforeEach(() => {
    h.charts.length = 0;
    h.heatmaps.length = 0;
});

afterEach(cleanup);

describe("FocusView frame", () => {
    it("is one sysinfo-focus element holding the area's charts", () => {
        const { container } = mount("net", NET_ROWS);
        const root = container.firstElementChild as HTMLElement;
        expect(root.className).toBe("sysinfo-focus");
        expect(Array.from(root.children).every((c) => c.getAttribute("data-testid") === "chart")).toBe(true);
    });

    it("shows nothing for an area it does not know", () => {
        const { container } = mount("gpu" as Area, NET_ROWS);
        expect((container.firstElementChild as HTMLElement).children.length).toBe(0);
        expect(charts().length).toBe(0);
    });

    it("hands every chart the pane's sizing, sync key and chart class", () => {
        mount("net", NET_ROWS, { targetLen: 301, intervalSecs: 5, syncKey: "sync-7" });
        expect(charts().length).toBe(3);
        for (const c of charts()) {
            expect(c.targetLen).toBe(301);
            expect(c.intervalSecs).toBe(5);
            expect(c.syncKey).toBe("sync-7");
            expect(c.class).toBe("sysinfo-focus-chart");
        }
    });

    it("switches between areas as the area prop changes", () => {
        const [area, setArea] = createSignal<Area>("net");
        const { getAllByTestId } = render(() => (
            <FocusView area={area()} plotData={MEM_ROWS} targetLen={121} intervalSecs={1} syncKey="s" />
        ));
        expect(getAllByTestId("chart").map((c) => c.textContent)).toEqual(["In above, out below", "In", "Out"]);
        setArea("mem");
        expect(getAllByTestId("chart").map((c) => c.textContent)).toEqual([
            "Used: 12.0 GB now, 10.0 GB – 14.0 GB",
            "Available: 20.0 GB now, 18.0 GB – 22.0 GB",
            "Committed: 10.0 GB now, 8.0 GB – 10.0 GB",
        ]);
    });
});

describe("FocusView CPU", () => {
    it("is one chart with the time axis when the sample has no per-core readings", () => {
        const rows = CPU_ROWS.map((r) => ({ ts: r.ts, cpu: r.cpu }) as DataItem);
        const { getAllByTestId } = mount("cpu", rows);
        expect(charts().length).toBe(1);
        expect(heatmaps().length).toBe(0);
        const c = charts()[0];
        expect(c.title).toBe("CPU: 40% now, 20% – 60%");
        expect(c.series).toEqual([{ key: "cpu", label: "CPU", color: "var(--sysinfo-cpu-color)" }]);
        expect(c.yRange([])).toEqual([0, 100]);
        expect(c.format(12.6)).toBe("13%");
        expect(c.timeAxis).toBe(true);
        expect(c.top).toBe(true);
        expect(c.plotData).toBe(rows);
        expect(getAllByTestId("chart").length).toBe(1);
    });

    it("is the average, the busiest core and the core heatmap when cores are reported", () => {
        const { container } = mount("cpu", CPU_ROWS);
        const root = container.firstElementChild as HTMLElement;
        expect(Array.from(root.children).map((c) => c.getAttribute("data-testid") ?? c.className)).toEqual([
            "chart",
            "chart",
            "sysinfo-focus-chart sysinfo-overview-cores",
        ]);
        expect(charts().length).toBe(2);
    });

    it("draws the average on top, with no time axis (the bottom chart labels it)", () => {
        mount("cpu", CPU_ROWS);
        const c = charts()[0];
        expect(c.title).toBe("CPU: 40% now, 20% – 60%");
        expect(c.series).toEqual([{ key: "cpu", label: "CPU", color: "var(--sysinfo-cpu-color)" }]);
        expect(c.yRange(CPU_ROWS)).toEqual([0, 100]);
        expect(c.format(50)).toBe("50%");
        expect(c.timeAxis).toBe(false);
        expect(c.top).toBe(true);
        expect(c.plotData).toBe(CPU_ROWS);
    });

    it("draws the busiest core against the average, over the rows with the busiest core added", () => {
        mount("cpu", CPU_ROWS);
        const c = charts()[1];
        const cores = ["cpu:0", "cpu:1", "cpu:2", "cpu:10"];
        expect(c.plotData).toEqual(withBusiestCore(CPU_ROWS, cores));
        expect(c.plotData.map((r) => r["cpu:busiest"])).toEqual([80, 60, 60]);
        expect(c.title).toBe("Busiest core: 60% now, 60% – 80%");
        expect(c.series).toEqual([
            { key: "cpu:busiest", label: "Busiest core", color: "var(--sysinfo-out-color)" },
            { key: "cpu", label: "Average", color: "var(--sysinfo-cpu-color)" },
        ]);
        expect(c.yRange(CPU_ROWS)).toEqual([0, 100]);
        expect(c.format(7)).toBe("7%");
        expect(c.timeAxis).toBe(true);
        expect(c.top).toBeUndefined();
    });

    it("titles the heatmap with the number of cores and hands it the rows, the cores and the colour", () => {
        const { getByTestId } = mount("cpu", CPU_ROWS, { targetLen: 61, intervalSecs: 2 });
        const panel = getByTestId("heatmap").parentElement as HTMLElement;
        expect(panel.className).toBe("sysinfo-focus-chart sysinfo-overview-cores");
        expect(panel.firstElementChild?.className).toBe("sysinfo-chart-title");
        expect(panel.firstElementChild?.textContent).toBe("Cores (4)");
        expect(heatmaps().length).toBe(1);
        const hm = heatmaps()[0];
        expect(hm.plotData).toBe(CPU_ROWS);
        expect(hm.cores).toEqual(["cpu:0", "cpu:1", "cpu:2", "cpu:10"]);
        expect(hm.color).toBe("var(--sysinfo-cpu-color)");
        expect(hm.targetLen).toBe(61);
        expect(hm.intervalSecs).toBe(2);
    });

    it("shows the busiest core and the heatmap for a single core", () => {
        const rows = [sample(0, { cpu: 10, "cpu:0": 10 }), sample(1, { cpu: 20, "cpu:0": 20 })];
        const { getByTestId } = mount("cpu", rows);
        expect(charts().length).toBe(2);
        expect(heatmaps().length).toBe(1);
        expect(heatmaps()[0].cores).toEqual(["cpu:0"]);
        expect(getByTestId("heatmap").parentElement?.firstElementChild?.textContent).toBe("Cores (1)");
    });

    it("reads the cores from the newest sample only", () => {
        const rows = [sample(0, { cpu: 10, "cpu:0": 10 }), sample(1, { cpu: 20 })];
        mount("cpu", rows);
        expect(charts().length).toBe(1);
        expect(heatmaps().length).toBe(0);
        expect(charts()[0].timeAxis).toBe(true);
    });

    it("shows the bare name of an empty window", () => {
        mount("cpu", []);
        expect(charts().length).toBe(1);
        expect(charts()[0].title).toBe("CPU");
        expect(charts()[0].timeAxis).toBe(true);
    });

    it("moves its titles with new samples without rebuilding the charts", () => {
        const [rows, setRows] = createSignal(CPU_ROWS);
        const { getAllByTestId } = render(() => (
            <FocusView area="cpu" plotData={rows()} targetLen={121} intervalSecs={1} syncKey="s" />
        ));
        const first = charts()[0];
        expect(first.title).toBe("CPU: 40% now, 20% – 60%");
        setRows([...CPU_ROWS, { ...CPU_ROWS[2], ts: base + 3000, cpu: 90 } as DataItem]);
        expect(charts().length).toBe(2);
        expect(first.title).toBe("CPU: 90% now, 20% – 90%");
        expect(first.plotData.length).toBe(4);
        expect(getAllByTestId("chart")[0].textContent).toBe("CPU: 90% now, 20% – 90%");
    });
});

describe("FocusView memory", () => {
    it("is used, available and committed, in that order, when the machine reports a commit limit", () => {
        const { getAllByTestId } = mount("mem", MEM_ROWS);
        expect(charts().length).toBe(3);
        expect(getAllByTestId("chart").map((c) => c.textContent)).toEqual([
            "Used: 12.0 GB now, 10.0 GB – 14.0 GB",
            "Available: 20.0 GB now, 18.0 GB – 22.0 GB",
            "Committed: 10.0 GB now, 8.0 GB – 10.0 GB",
        ]);
    });

    it("draws used on top, scaled as the used-memory metric", () => {
        mount("mem", MEM_ROWS);
        const c = charts()[0];
        expect(c.series).toEqual([{ key: "mem:used", label: "Used", color: "var(--sysinfo-mem-color)" }]);
        expect(c.yRange(MEM_ROWS)).toEqual(yRange(MEM_ROWS, "mem:used", DefaultPlotMeta["mem:used"]));
        expect(c.format).toBe(formatGB);
        expect(c.timeAxis).toBe(false);
        expect(c.top).toBe(true);
        expect(c.plotData).toBe(MEM_ROWS);
    });

    it("scales available by its own readings against the used-memory metric", () => {
        mount("mem", MEM_ROWS);
        const c = charts()[1];
        expect(c.series).toEqual([{ key: "mem:available", label: "Available", color: "var(--sysinfo-mem-color)" }]);
        expect(c.yRange(MEM_ROWS)).toEqual(yRange(MEM_ROWS, "mem:available", DefaultPlotMeta["mem:used"]));
        expect(c.yRange(MEM_ROWS)).not.toEqual(yRange(MEM_ROWS, "mem:used", DefaultPlotMeta["mem:used"]));
        expect(c.format).toBe(formatGB);
        expect(c.timeAxis).toBe(false);
        expect(c.top).toBeUndefined();
    });

    it("draws committed last, on the time axis, against its limit", () => {
        mount("mem", MEM_ROWS);
        const c = charts()[2];
        expect(c.series).toEqual([{ key: "mem:commit:used", label: "Committed", color: "var(--sysinfo-out-color)" }]);
        expect(c.yRange(MEM_ROWS)).toEqual([0, 32]);
        expect(c.format).toBe(formatGB);
        expect(c.timeAxis).toBe(true);
        expect(c.top).toBeUndefined();
    });

    it("leaves committed out, and puts the time axis on available, without a commit limit", () => {
        const rows = MEM_ROWS.map((r) => ({ ...r, "mem:commit:total": 0 }) as DataItem);
        mount("mem", rows);
        expect(charts().length).toBe(2);
        expect(charts()[1].title).toBe("Available: 20.0 GB now, 18.0 GB – 22.0 GB");
        expect(charts()[1].timeAxis).toBe(true);
        expect(charts()[0].timeAxis).toBe(false);
    });

    it("leaves committed out when the newest sample has no commit limit at all", () => {
        mount("mem", [MEM_ROWS[0], sample(1, { "mem:used": 1, "mem:available": 2 })]);
        expect(charts().length).toBe(2);
        expect(charts()[1].timeAxis).toBe(true);
    });

    it("leaves committed out for an empty window", () => {
        mount("mem", []);
        expect(charts().length).toBe(2);
        expect(charts().map((c) => c.title)).toEqual(["Used", "Available"]);
    });

    it("brings committed in when the limit appears", () => {
        const [rows, setRows] = createSignal<DataItem[]>([sample(0, { "mem:used": 1, "mem:available": 2 })]);
        const { getAllByTestId } = render(() => (
            <FocusView area="mem" plotData={rows()} targetLen={121} intervalSecs={1} syncKey="s" />
        ));
        expect(getAllByTestId("chart").length).toBe(2);
        setRows(MEM_ROWS);
        expect(getAllByTestId("chart").length).toBe(3);
    });
});

describe("FocusView disk", () => {
    it("is the mirrored read/write chart, read, write, then the volumes", () => {
        const { container } = mount("disk", DISK_ROWS);
        const root = container.firstElementChild as HTMLElement;
        expect(Array.from(root.children).map((c) => c.getAttribute("data-testid") ?? c.tagName)).toEqual([
            "chart",
            "chart",
            "chart",
            "UL",
        ]);
        expect(charts().map((c) => c.title)).toEqual([
            "Read above, write below",
            "Read: 1.0 MB/s now, 500 KB/s – 2.0 MB/s",
            "Write: 300 KB/s now, 100 KB/s – 300 KB/s",
        ]);
    });

    it("draws read above and write below the axis on one mirrored scale", () => {
        mount("disk", DISK_ROWS);
        const c = charts()[0];
        expect(c.series).toEqual([
            { key: "disk:read", label: "Read", color: "var(--sysinfo-in-color)" },
            { key: "disk:write", label: "Write", color: "var(--sysinfo-out-color)", below: true },
        ]);
        expect(c.yRange(DISK_ROWS)).toEqual(mirroredRange(DISK_ROWS, "disk:read", "disk:write", 1));
        expect(c.format).toBe(formatRate);
        expect(c.timeAxis).toBe(false);
        expect(c.top).toBe(true);
    });

    it("draws read alone, scaled as the disk read metric", () => {
        mount("disk", DISK_ROWS);
        const c = charts()[1];
        expect(c.series).toEqual([{ key: "disk:read", label: "Read", color: "var(--sysinfo-in-color)" }]);
        expect(c.yRange(DISK_ROWS)).toEqual(yRange(DISK_ROWS, "disk:read", DefaultPlotMeta["disk:read"]));
        expect(c.format).toBe(formatRate);
        expect(c.timeAxis).toBe(false);
        expect(c.top).toBeUndefined();
    });

    it("draws write last, on the time axis, scaled as the disk write metric", () => {
        mount("disk", DISK_ROWS);
        const c = charts()[2];
        expect(c.series).toEqual([{ key: "disk:write", label: "Write", color: "var(--sysinfo-out-color)" }]);
        expect(c.yRange(DISK_ROWS)).toEqual(yRange(DISK_ROWS, "disk:write", DefaultPlotMeta["disk:write"]));
        expect(c.format).toBe(formatRate);
        expect(c.timeAxis).toBe(true);
    });

    it("lists the newest sample's volumes by mount, with their free and total space", () => {
        const { container } = mount("disk", DISK_ROWS);
        const list = container.querySelector("ul.sysinfo-focus-volumes") as HTMLElement;
        expect(list.getAttribute("aria-label")).toBe("Volumes");
        const items = Array.from(list.querySelectorAll("li"));
        expect(items.map((li) => li.querySelector(".sysinfo-focus-volume-name")?.textContent)).toEqual(["/a", "/b"]);
        expect(items.map((li) => li.textContent)).toEqual([
            "/a100.0 GB free of 500.0 GB",
            "/b50.0 GB free of 200.0 GB",
        ]);
    });

    it("lists a single volume", () => {
        const rows = [
            sample(0, { "disk:read": 1, "disk:write": 1, "disk:vol:/:free_gb": 20, "disk:vol:/:total_gb": 80 }),
        ];
        const { container } = mount("disk", rows);
        const items = Array.from(container.querySelectorAll("ul.sysinfo-focus-volumes li"));
        expect(items.map((li) => li.textContent)).toEqual(["/20.0 GB free of 80.0 GB"]);
    });

    it("scales a quiet mirrored chart to at least 1 MB/s each way", () => {
        const quiet = [sample(0, { "disk:read": 0.01, "disk:write": 0.02 })];
        mount("disk", quiet);
        const [low, high] = charts()[0].yRange(quiet);
        expect(high).toBeGreaterThanOrEqual(1);
        expect(low).toBe(-high);
        expect(charts()[0].yRange(quiet)).toEqual(mirroredRange(quiet, "disk:read", "disk:write", 1));
    });

    it("has no volume list when the newest sample carries none", () => {
        const { container } = mount("disk", [DISK_ROWS[2], sample(3, { "disk:read": 1, "disk:write": 1 })]);
        expect(container.querySelector("ul")).toBeNull();
        expect(charts().length).toBe(3);
    });
});

describe("FocusView network", () => {
    it("is the mirrored in/out chart, in, then out", () => {
        const { container } = mount("net", NET_ROWS);
        const root = container.firstElementChild as HTMLElement;
        expect(root.children.length).toBe(3);
        expect(charts().map((c) => c.title)).toEqual([
            "In above, out below",
            "In: 2.0 MB/s now, 1.0 MB/s – 3.0 MB/s",
            "Out: 100 KB/s now, 100 KB/s – 400 KB/s",
        ]);
    });

    it("draws in above and out below the axis on one mirrored scale", () => {
        mount("net", NET_ROWS);
        const c = charts()[0];
        expect(c.series).toEqual([
            { key: "net:bytesrecv", label: "In", color: "var(--sysinfo-in-color)" },
            { key: "net:bytessent", label: "Out", color: "var(--sysinfo-out-color)", below: true },
        ]);
        expect(c.yRange(NET_ROWS)).toEqual(mirroredRange(NET_ROWS, "net:bytesrecv", "net:bytessent", 1));
        expect(c.format).toBe(formatRate);
        expect(c.timeAxis).toBe(false);
        expect(c.top).toBe(true);
    });

    it("draws in alone, scaled as the network received metric", () => {
        mount("net", NET_ROWS);
        const c = charts()[1];
        expect(c.series).toEqual([{ key: "net:bytesrecv", label: "In", color: "var(--sysinfo-in-color)" }]);
        expect(c.yRange(NET_ROWS)).toEqual(yRange(NET_ROWS, "net:bytesrecv", DefaultPlotMeta["net:bytesrecv"]));
        expect(c.format).toBe(formatRate);
        expect(c.timeAxis).toBe(false);
        expect(c.top).toBeUndefined();
    });

    it("draws out last, on the time axis, scaled as the network sent metric", () => {
        mount("net", NET_ROWS);
        const c = charts()[2];
        expect(c.series).toEqual([{ key: "net:bytessent", label: "Out", color: "var(--sysinfo-out-color)" }]);
        expect(c.yRange(NET_ROWS)).toEqual(yRange(NET_ROWS, "net:bytessent", DefaultPlotMeta["net:bytessent"]));
        expect(c.format).toBe(formatRate);
        expect(c.timeAxis).toBe(true);
    });

    it("scales a quiet mirrored chart to at least 1 MB/s each way", () => {
        const quiet = [sample(0, { "net:bytesrecv": 0.01, "net:bytessent": 0.02 })];
        mount("net", quiet);
        const [low, high] = charts()[0].yRange(quiet);
        expect(high).toBeGreaterThanOrEqual(1);
        expect(low).toBe(-high);
        expect(charts()[0].yRange(quiet)).toEqual(mirroredRange(quiet, "net:bytesrecv", "net:bytessent", 1));
    });

    it("shows the bare names of an empty window", () => {
        mount("net", []);
        expect(charts().map((c) => c.title)).toEqual(["In above, out below", "In", "Out"]);
    });
});
