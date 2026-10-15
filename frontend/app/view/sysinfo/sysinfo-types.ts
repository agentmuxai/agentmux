// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { AreaChoice } from "./sysinfo-overview-data";

export const DefaultNumPoints = 120;

export type DataItem = {
    ts: number;
    [k: string]: number;
};

function defaultCpuMeta(name: string): TimeSeriesMeta {
    return {
        name: name,
        label: "%",
        miny: 0,
        maxy: 100,
        color: "var(--sysinfo-cpu-color)",
        decimalPlaces: 0,
    };
}

function defaultMemMeta(name: string, maxY: string): TimeSeriesMeta {
    return {
        name: name,
        label: "GB",
        miny: 0,
        // `mem:total` is a hard cap (can't physically use more than total
        // RAM) — autoMaxY zooms in tighter than that when actual usage sits
        // well below it, without ever showing headroom that doesn't exist.
        // See docs/reports/REPORT_SYSINFO_COMBINED_CHART_RESEARCH_2026_08_17.md.
        maxy: maxY,
        autoMaxY: true,
        // Floor in GB, not a fraction of total — keeps the axis from
        // zooming in on sub-1GB noise on a mostly-idle machine.
        autoMaxYFloor: 1,
        color: "var(--sysinfo-mem-color)",
        decimalPlaces: 1,
    };
}

function defaultNetMeta(name: string, color = "var(--sysinfo-in-color)"): TimeSeriesMeta {
    return {
        name: name,
        label: "MB/s",
        miny: 0,
        // No hard cap (unlike CPU%/mem:total, there's no real ceiling for
        // throughput) — was a fixed 100, which clipped real spikes above it
        // and wasted most of the chart when idle traffic sat at 0-5 MB/s.
        // See docs/reports/REPORT_SYSINFO_COMBINED_CHART_RESEARCH_2026_08_17.md.
        autoMaxY: true,
        autoMaxYFloor: 1,
        color,
        decimalPlaces: 2,
    };
}

function defaultDiskMeta(name: string, color = "var(--sysinfo-in-color)"): TimeSeriesMeta {
    return {
        name: name,
        label: "MB/s",
        miny: 0,
        // Same reasoning as defaultNetMeta above — disk throughput has no
        // real ceiling either.
        autoMaxY: true,
        autoMaxYFloor: 1,
        color,
        decimalPlaces: 2,
    };
}

/** The plot type of the Advanced views (CPU, Memory, Disk, Network, Everything with their controls), as saved. */
export const AreaPlotType = "Overview";

/** The plot type a pane shows until one is chosen: simple CPU, one chart, cheap to open. */
export const DefaultPlotType = "CPU";

export const PlotTypes: Record<string, (dataItem: DataItem) => string[]> = {
    // CPU, memory, network, disk and every core at once (sysinfo-overview.tsx);
    // its metrics list is only what the other plot types' code expects.
    Overview: function (dataItem: DataItem): Array<string> {
        return ["cpu"];
    },
    CPU: function (dataItem: DataItem): Array<string> {
        return ["cpu"];
    },
    Mem: function (dataItem: DataItem): Array<string> {
        return ["mem:used"];
    },
    "CPU + Mem": function (dataItem: DataItem): Array<string> {
        return ["cpu", "mem:used"];
    },
    Net: function (dataItem: DataItem): Array<string> {
        return ["net:bytestotal"];
    },
    "Net (Sent/Recv)": function (dataItem: DataItem): Array<string> {
        return ["net:bytessent", "net:bytesrecv"];
    },
    "CPU + Mem + Net": function (dataItem: DataItem): Array<string> {
        return ["cpu", "mem:used", "net:bytestotal"];
    },
    "Disk I/O": function (dataItem: DataItem): Array<string> {
        return ["disk:total"];
    },
    "Disk I/O (R/W)": function (dataItem: DataItem): Array<string> {
        return ["disk:read", "disk:write"];
    },
    "All CPU": function (dataItem: DataItem): Array<string> {
        return Object.keys(dataItem)
            .filter((item) => item.startsWith("cpu") && item != "cpu")
            .sort((a, b) => {
                const valA = parseInt(a.replace("cpu:", ""));
                const valB = parseInt(b.replace("cpu:", ""));
                return valA - valB;
            });
    },
};

/**
 * The simple views: one plain chart per measure, no controls, offered under
 * "Simple" in the pane's menus. `type` is the plot type saved in the pane's
 * meta (`sysinfo:type`), kept as it was so saved panes still open on it;
 * `label` is how the menus and the pane's title name it, `short` how the
 * control row does, and `area` whose figures its table lists.
 */
export const SIMPLE_VIEWS: { type: string; label: string; short: string; area: AreaChoice }[] = [
    { type: "CPU", label: "CPU", short: "CPU", area: "cpu" },
    { type: "All CPU", label: "All cores", short: "Cores", area: "cpu" },
    { type: "Mem", label: "Memory", short: "Memory", area: "mem" },
    { type: "Net", label: "Network", short: "Network", area: "net" },
    { type: "Net (Sent/Recv)", label: "Network sent and received", short: "Net in/out", area: "net" },
    { type: "Disk I/O", label: "Disk", short: "Disk", area: "disk" },
    { type: "Disk I/O (R/W)", label: "Disk read and write", short: "Disk r/w", area: "disk" },
    { type: "CPU + Mem", label: "CPU and memory", short: "CPU+Mem", area: "everything" },
    { type: "CPU + Mem + Net", label: "CPU, memory and network", short: "CPU+Mem+Net", area: "everything" },
];

export const DefaultPlotMeta: Record<string, TimeSeriesMeta> = {
    cpu: defaultCpuMeta("CPU"),
    "mem:total": defaultMemMeta("Memory total", "mem:total"),
    "mem:used": defaultMemMeta("Memory used", "mem:total"),
    "mem:free": defaultMemMeta("Memory free", "mem:total"),
    "mem:available": defaultMemMeta("Memory available", "mem:total"),
    "net:bytessent": defaultNetMeta("Network out", "var(--sysinfo-out-color)"),
    "net:bytesrecv": defaultNetMeta("Network in"),
    "net:bytestotal": defaultNetMeta("Network"),
    "disk:read": defaultDiskMeta("Disk read"),
    "disk:write": defaultDiskMeta("Disk write", "var(--sysinfo-out-color)"),
    "disk:total": defaultDiskMeta("Disk"),
};
for (let i = 0; i < 32; i++) {
    DefaultPlotMeta[`cpu:${i}`] = defaultCpuMeta(`Core ${i}`);
}
