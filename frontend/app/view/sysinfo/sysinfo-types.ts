// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

export const DefaultNumPoints = 120;

export type DataItem = {
    ts: number;
    [k: string]: number;
};

// Use a loose type to avoid circular dependency with sysinfo-model.ts.
// The model is typed as SysinfoViewModel at usage sites.
export type SysinfoViewProps = {
    blockId: string;
    model: any;
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
        maxy: maxY,
        color: "var(--sysinfo-mem-color)",
        decimalPlaces: 1,
    };
}

function defaultNetMeta(name: string): TimeSeriesMeta {
    return {
        name: name,
        label: "MB/s",
        miny: 0,
        maxy: 100,
        color: "var(--sysinfo-net-color)",
        decimalPlaces: 2,
    };
}

function defaultDiskMeta(name: string): TimeSeriesMeta {
    return {
        name: name,
        label: "MB/s",
        miny: 0,
        maxy: 100,
        color: "var(--sysinfo-net-color)",
        decimalPlaces: 2,
    };
}

export const PlotTypes: Record<string, (dataItem: DataItem) => string[]> = {
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

export const DefaultPlotMeta: Record<string, TimeSeriesMeta> = {
    cpu: defaultCpuMeta("CPU %"),
    "mem:total": defaultMemMeta("Memory Total", "mem:total"),
    "mem:used": defaultMemMeta("Memory Used", "mem:total"),
    "mem:free": defaultMemMeta("Memory Free", "mem:total"),
    "mem:available": defaultMemMeta("Memory Available", "mem:total"),
    "net:bytessent": defaultNetMeta("Network Sent"),
    "net:bytesrecv": defaultNetMeta("Network Recv"),
    "net:bytestotal": defaultNetMeta("Network Total"),
    "disk:read": defaultDiskMeta("Disk Read"),
    "disk:write": defaultDiskMeta("Disk Write"),
    "disk:total": defaultDiskMeta("Disk Total"),
};
for (let i = 0; i < 32; i++) {
    DefaultPlotMeta[`cpu:${i}`] = defaultCpuMeta(`Core ${i}`);
}
