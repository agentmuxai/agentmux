// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Sysinfo's Overview: the machine at a glance, then over time.
 *
 * - One row of controls: every area or one alone (`sysinfo-focus.tsx`), the
 *   window (1 to 15 minutes), pause, and a table of the figures instead of the
 *   charts. In a narrow pane the area buttons take a row of their own.
 * - A tile per area (CPU, memory, network, disk): its value now, its low and
 *   high over the window, and a sparkline.
 * - Charts, one per measure on its own scale (never two y axes), sharing one
 *   time axis and one crosshair: CPU, every core as a heatmap, memory, and
 *   network and disk with what comes in above the axis and what goes out below.
 *
 * docs/reports/REPORT_SYSINFO_REWRITE_AND_MODERN_MONITOR_2026_10_13.md section 5.3.
 */

import type { JSX } from "solid-js";
import { createMemo, createSignal, For, Show } from "solid-js";

import { IconButton, SegmentedControl } from "@/app/element/ui";

import "./sysinfo-overview.scss";

import { formatValue, type SeriesSpec, yRange } from "./sysinfo-chart";
import { FocusView } from "./sysinfo-focus";
import { CoreHeatmap } from "./sysinfo-heatmap";
import {
    coreKeys,
    FOCUS_CHOICES,
    type FocusArea,
    formatGB,
    formatRate,
    mirroredRange,
    windowChoices,
    windowStats,
    type WindowStats,
} from "./sysinfo-overview-data";
import { TimeChart } from "./sysinfo-plot";
import { type DataItem, DefaultPlotMeta } from "./sysinfo-types";

const CPU = "var(--sysinfo-cpu-color)";
const MEM = "var(--sysinfo-mem-color)";
const IN = "var(--sysinfo-in-color)";
const OUT = "var(--sysinfo-out-color)";

type OverviewProps = {
    plotData: DataItem[];
    targetLen: number;
    intervalSecs: number;
    /** The window shown now, in seconds, and a way to change it. */
    windowSecs: number;
    onWindow: (secs: number) => void;
    paused: boolean;
    onPause: (paused: boolean) => void;
    /** The area shown alone, or `null` for every area, and a way to change it. */
    focus: FocusArea | null;
    onFocus: (area: FocusArea | null) => void;
    syncKey: string;
};

type Stats = Record<"cpu" | "mem" | "total" | "recv" | "sent" | "read" | "write", WindowStats | null>;

/**
 * One tile. Its figures are read from the window's stats as they change; the
 * tile itself, and its sparkline, stay built.
 */
interface Tile {
    name: string;
    value: (s: Stats) => string;
    range: (s: Stats) => string;
    sparkKey: string;
    color: string;
    format: (v: number) => string;
    range0: (rows: DataItem[]) => [number, number];
}

/** Both directions' peaks over the window ("peak ↓ 2.5 MB/s  ↑ 400 KB/s"), leaving out one with no readings. */
function peaks(inLabel: string, inStats: WindowStats | null, outLabel: string, outStats: WindowStats | null): string {
    const parts = [
        inStats ? `${inLabel} ${formatRate(inStats.max)}` : "",
        outStats ? `${outLabel} ${formatRate(outStats.max)}` : "",
    ].filter(Boolean);
    return parts.length ? `peak ${parts.join("  ")}` : "";
}

function lowHigh(s: WindowStats | null, f: (v: number) => string): string {
    return s ? `${f(s.min)} – ${f(s.max)}` : "";
}

export function Overview(props: OverviewProps): JSX.Element {
    const [table, setTable] = createSignal(false);
    const rows = () => props.plotData;
    const latest = () => rows()[rows().length - 1];
    const cores = createMemo(() => coreKeys(latest()));
    const pct = (v: number) => formatValue(v, 0, "%");

    const stats = createMemo((): Stats => ({
        cpu: windowStats(rows(), "cpu"),
        mem: windowStats(rows(), "mem:used"),
        total: windowStats(rows(), "mem:total"),
        recv: windowStats(rows(), "net:bytesrecv"),
        sent: windowStats(rows(), "net:bytessent"),
        read: windowStats(rows(), "disk:read"),
        write: windowStats(rows(), "disk:write"),
    }));

    // Built once: a list made again on every sample would make <For> build every
    // tile and its sparkline again each second.
    const tiles: Tile[] = [
        {
            name: "CPU",
            value: (s) => (s.cpu ? pct(s.cpu.now) : "–"),
            range: (s) => lowHigh(s.cpu, pct),
            sparkKey: "cpu",
            color: CPU,
            format: pct,
            range0: () => [0, 100],
        },
        {
            name: "Memory",
            value: (s) => (s.mem ? `${formatGB(s.mem.now)}${s.total ? ` of ${formatGB(s.total.now)}` : ""}` : "–"),
            range: (s) => lowHigh(s.mem, formatGB),
            sparkKey: "mem:used",
            color: MEM,
            format: formatGB,
            range0: (r) => yRange(r, "mem:used", DefaultPlotMeta["mem:used"]),
        },
        {
            name: "Network",
            value: (s) => (s.recv || s.sent ? `↓ ${formatRate(s.recv?.now)}  ↑ ${formatRate(s.sent?.now)}` : "–"),
            range: (s) => peaks("↓", s.recv, "↑", s.sent),
            sparkKey: "net:bytesrecv",
            color: IN,
            format: formatRate,
            range0: (r) => yRange(r, "net:bytesrecv", DefaultPlotMeta["net:bytesrecv"]),
        },
        {
            name: "Disk",
            value: (s) => (s.read || s.write ? `R ${formatRate(s.read?.now)}  W ${formatRate(s.write?.now)}` : "–"),
            range: (s) => peaks("R", s.read, "W", s.write),
            sparkKey: "disk:read",
            color: IN,
            format: formatRate,
            range0: (r) => yRange(r, "disk:read", DefaultPlotMeta["disk:read"]),
        },
    ];

    const netSeries: SeriesSpec[] = [
        { key: "net:bytesrecv", label: "In", color: IN },
        { key: "net:bytessent", label: "Out", color: OUT, below: true },
    ];
    const diskSeries: SeriesSpec[] = [
        { key: "disk:read", label: "Read", color: IN },
        { key: "disk:write", label: "Write", color: OUT, below: true },
    ];

    const tableRows = createMemo(() => {
        const s = stats();
        const line = (name: string, st: WindowStats | null, f: (v: number) => string) => ({
            name,
            now: st ? f(st.now) : "–",
            min: st ? f(st.min) : "–",
            max: st ? f(st.max) : "–",
            avg: st ? f(st.avg) : "–",
        });
        const cpu = [
            line("CPU", s.cpu, pct),
            ...cores().map((k) => line(`Core ${k.slice(4)}`, windowStats(rows(), k), pct)),
        ];
        const mem = [
            line("Memory used", s.mem, formatGB),
            line("Memory available", windowStats(rows(), "mem:available"), formatGB),
            line("Memory committed", windowStats(rows(), "mem:commit:used"), formatGB),
        ];
        const net = [line("Network in", s.recv, formatRate), line("Network out", s.sent, formatRate)];
        const disk = [line("Disk read", s.read, formatRate), line("Disk write", s.write, formatRate)];
        // An area alone lists only its own measures.
        switch (props.focus) {
            case "cpu":
                return cpu;
            case "mem":
                return mem.filter((r) => r.now !== "–");
            case "net":
                return net;
            case "disk":
                return disk;
            default:
                return [cpu[0], mem[0], ...net, ...disk, ...cpu.slice(1)];
        }
    });

    return (
        <div class="sysinfo-overview">
            <div class="sysinfo-overview-controls">
                <SegmentedControl
                    ariaLabel="Area"
                    density="compact"
                    options={FOCUS_CHOICES.map((c) => ({ value: c.value, label: c.label }))}
                    value={props.focus ?? "overview"}
                    onChange={(v) => props.onFocus(v === "overview" ? null : (v as FocusArea))}
                />
                <SegmentedControl
                    ariaLabel="Time window"
                    density="compact"
                    options={windowChoices(props.intervalSecs, props.windowSecs).map((c) => ({
                        value: String(c.secs),
                        label: c.label,
                    }))}
                    value={String(props.windowSecs)}
                    onChange={(v) => props.onWindow(Number(v))}
                />
                <Show when={props.paused}>
                    <span class="sysinfo-overview-paused">Paused</span>
                </Show>
                <div class="sysinfo-overview-spacer" />
                <IconButton
                    icon={props.paused ? "play" : "pause"}
                    label={props.paused ? "Resume" : "Pause"}
                    pressed={props.paused}
                    onClick={() => props.onPause(!props.paused)}
                />
                <IconButton
                    icon="table"
                    label={table() ? "Show charts" : "Show as a table"}
                    pressed={table()}
                    onClick={() => setTable(!table())}
                />
            </div>

            <Show when={!props.focus}>
                <div class="sysinfo-overview-tiles">
                    <For each={tiles}>
                        {(t) => (
                            <div class="sysinfo-tile">
                                <div class="sysinfo-tile-name">{t.name}</div>
                                <div class="sysinfo-tile-value">{t.value(stats())}</div>
                                <div class="sysinfo-tile-range">{t.range(stats())}</div>
                                <TimeChart
                                    plotData={rows()}
                                    series={[{ key: t.sparkKey, label: t.name, color: t.color }]}
                                    yRange={t.range0}
                                    format={t.format}
                                    targetLen={props.targetLen}
                                    intervalSecs={props.intervalSecs}
                                    sparkline
                                    class="sysinfo-tile-spark"
                                />
                            </div>
                        )}
                    </For>
                </div>
            </Show>

            <Show
                when={!table()}
                fallback={
                    <table class="sysinfo-overview-table">
                        <thead>
                            <tr>
                                <th scope="col">Measure</th>
                                <th scope="col">Now</th>
                                <th scope="col">Low</th>
                                <th scope="col">High</th>
                                <th scope="col">Average</th>
                            </tr>
                        </thead>
                        <tbody>
                            <For each={tableRows()}>
                                {(r) => (
                                    <tr>
                                        <th scope="row">{r.name}</th>
                                        <td>{r.now}</td>
                                        <td>{r.min}</td>
                                        <td>{r.max}</td>
                                        <td>{r.avg}</td>
                                    </tr>
                                )}
                            </For>
                        </tbody>
                    </table>
                }
            >
                <Show
                    when={!props.focus}
                    fallback={
                        <FocusView
                            area={props.focus as FocusArea}
                            plotData={rows()}
                            targetLen={props.targetLen}
                            intervalSecs={props.intervalSecs}
                            syncKey={props.syncKey}
                        />
                    }
                >
                    <div class="sysinfo-overview-charts">
                        <TimeChart
                            plotData={rows()}
                            series={[{ key: "cpu", label: "CPU", color: CPU }]}
                            yRange={() => [0, 100]}
                            format={pct}
                            targetLen={props.targetLen}
                            intervalSecs={props.intervalSecs}
                            syncKey={props.syncKey}
                            title="CPU"
                            class="sysinfo-overview-wide"
                        />
                        <Show when={cores().length > 0}>
                            <div class="sysinfo-overview-wide sysinfo-overview-cores">
                                <div class="sysinfo-chart-title">{`Cores (${cores().length})`}</div>
                                <CoreHeatmap
                                    plotData={rows()}
                                    cores={cores()}
                                    color={CPU}
                                    targetLen={props.targetLen}
                                    intervalSecs={props.intervalSecs}
                                />
                            </div>
                        </Show>
                        <TimeChart
                            plotData={rows()}
                            series={[{ key: "mem:used", label: "Memory", color: MEM }]}
                            yRange={(r) => yRange(r, "mem:used", DefaultPlotMeta["mem:used"])}
                            format={formatGB}
                            targetLen={props.targetLen}
                            intervalSecs={props.intervalSecs}
                            syncKey={props.syncKey}
                            title="Memory used"
                        />
                        <TimeChart
                            plotData={rows()}
                            series={netSeries}
                            yRange={(r) => mirroredRange(r, "net:bytesrecv", "net:bytessent", 1)}
                            format={formatRate}
                            targetLen={props.targetLen}
                            intervalSecs={props.intervalSecs}
                            syncKey={props.syncKey}
                            title="Network: in above, out below"
                        />
                        <TimeChart
                            plotData={rows()}
                            series={diskSeries}
                            yRange={(r) => mirroredRange(r, "disk:read", "disk:write", 1)}
                            format={formatRate}
                            targetLen={props.targetLen}
                            intervalSecs={props.intervalSecs}
                            syncKey={props.syncKey}
                            title="Disk: read above, write below"
                            class="sysinfo-overview-wide"
                        />
                    </div>
                </Show>
            </Show>
        </div>
    );
}
