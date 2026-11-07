// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One area of the Overview shown alone (CPU, memory, disk or network), for a
 * pane too small for every area at once: at most three charts, sharing the
 * pane's height, each titled with its figure now and its low and high over
 * the window. No tiles; the titles carry the figures. Only the bottom chart
 * labels the time; the crosshair reads it on the others.
 * docs/reports/REPORT_SYSINFO_REWRITE_AND_MODERN_MONITOR_2026_10_13.md section 5.3.
 */

import type { JSX } from "solid-js";
import { createMemo, For, Match, Show, Switch } from "solid-js";

import { formatValue, type SeriesSpec, yRange } from "./sysinfo-chart";
import { CoreHeatmap } from "./sysinfo-heatmap";
import {
    BUSIEST_CORE,
    coreKeys,
    figures,
    type FocusArea,
    formatGB,
    formatRate,
    mirroredRange,
    titled,
    volumes,
    withBusiestCore,
} from "./sysinfo-overview-data";
import { TimeChart } from "./sysinfo-plot-lazy";
import { type DataItem, DefaultPlotMeta } from "./sysinfo-types";

const CPU = "var(--sysinfo-cpu-color)";
const MEM = "var(--sysinfo-mem-color)";
const IN = "var(--sysinfo-in-color)";
const OUT = "var(--sysinfo-out-color)";

const pct = (v: number) => formatValue(v, 0, "%");

type FocusProps = {
    area: FocusArea;
    plotData: DataItem[];
    targetLen: number;
    intervalSecs: number;
    syncKey: string;
};


// Committed memory is read against its limit, so its scale ends there.
const commitMeta = { ...DefaultPlotMeta["mem:used"], maxy: "mem:commit:total", autoMaxY: false } as TimeSeriesMeta;

function createFocusState(props: FocusProps) {
    const rows = () => props.plotData;
    const latest = () => rows()[rows().length - 1];
    const cores = createMemo(() => coreKeys(latest()));
    const cpuRows = createMemo(() => (props.area === "cpu" ? withBusiestCore(rows(), cores()) : rows()));
    const hasCommit = () => latest()?.["mem:commit:total"] > 0;
    return { props, rows, latest, cores, cpuRows, hasCommit };
}

type FocusState = ReturnType<typeof createFocusState>;

// The title is read inside the chart, so new samples change its text, not the chart.
function focusChart(
    s: FocusState,
    title: () => string,
    series: SeriesSpec[],
    range: (r: DataItem[]) => [number, number],
    format: (v: number) => string,
    opts: { data?: () => DataItem[]; timeAxis?: () => boolean; top?: boolean } = {}
) {
    return (
        <TimeChart
            plotData={(opts.data ?? s.rows)()}
            series={series}
            yRange={range}
            format={format}
            targetLen={s.props.targetLen}
            intervalSecs={s.props.intervalSecs}
            syncKey={s.props.syncKey}
            title={title()}
            timeAxis={opts.timeAxis?.() ?? false}
            top={opts.top}
            class="sysinfo-focus-chart"
        />
    );
}

function CpuCharts({ s }: { s: FocusState }): JSX.Element {
    return (
        <>
            {focusChart(
                s,
                () => titled("CPU", figures(s.rows(), "cpu", pct)),
                [{ key: "cpu", label: "CPU", color: CPU }],
                () => [0, 100],
                pct,
                { timeAxis: () => s.cores().length === 0, top: true }
            )}
            <Show when={s.cores().length > 0}>
                {focusChart(
                    s,
                    () => titled("Busiest core", figures(s.cpuRows(), BUSIEST_CORE, pct)),
                    [
                        { key: BUSIEST_CORE, label: "Busiest core", color: OUT },
                        { key: "cpu", label: "Average", color: CPU },
                    ],
                    () => [0, 100],
                    pct,
                    { data: s.cpuRows, timeAxis: () => true }
                )}
                <div class="sysinfo-focus-chart sysinfo-overview-cores">
                    <div class="sysinfo-chart-title">{`Cores (${s.cores().length})`}</div>
                    <CoreHeatmap
                        plotData={s.rows()}
                        cores={s.cores()}
                        color={CPU}
                        targetLen={s.props.targetLen}
                        intervalSecs={s.props.intervalSecs}
                    />
                </div>
            </Show>
        </>
    );
}

function MemCharts({ s }: { s: FocusState }): JSX.Element {
    return (
        <>
            {focusChart(
                s,
                () => titled("Used", figures(s.rows(), "mem:used", formatGB)),
                [{ key: "mem:used", label: "Used", color: MEM }],
                (r) => yRange(r, "mem:used", DefaultPlotMeta["mem:used"]),
                formatGB,
                { top: true }
            )}
            {focusChart(
                s,
                () => titled("Available", figures(s.rows(), "mem:available", formatGB)),
                [{ key: "mem:available", label: "Available", color: MEM }],
                (r) => yRange(r, "mem:available", DefaultPlotMeta["mem:used"]),
                formatGB,
                { timeAxis: () => !s.hasCommit() }
            )}
            <Show when={s.hasCommit()}>
                {focusChart(
                    s,
                    () => titled("Committed", figures(s.rows(), "mem:commit:used", formatGB)),
                    [{ key: "mem:commit:used", label: "Committed", color: OUT }],
                    (r) => yRange(r, "mem:commit:used", commitMeta),
                    formatGB,
                    { timeAxis: () => true }
                )}
            </Show>
        </>
    );
}

function DiskCharts({ s }: { s: FocusState }): JSX.Element {
    return (
        <>
            {focusChart(
                s,
                () => "Read above, write below",
                [
                    { key: "disk:read", label: "Read", color: IN },
                    { key: "disk:write", label: "Write", color: OUT, below: true },
                ],
                (r) => mirroredRange(r, "disk:read", "disk:write", 1),
                formatRate,
                { top: true }
            )}
            {focusChart(
                s,
                () => titled("Read", figures(s.rows(), "disk:read", formatRate)),
                [{ key: "disk:read", label: "Read", color: IN }],
                (r) => yRange(r, "disk:read", DefaultPlotMeta["disk:read"]),
                formatRate
            )}
            {focusChart(
                s,
                () => titled("Write", figures(s.rows(), "disk:write", formatRate)),
                [{ key: "disk:write", label: "Write", color: OUT }],
                (r) => yRange(r, "disk:write", DefaultPlotMeta["disk:write"]),
                formatRate,
                { timeAxis: () => true }
            )}
            <Show when={volumes(s.latest()).length > 0}>
                <ul class="sysinfo-focus-volumes" aria-label="Volumes">
                    <For each={volumes(s.latest())}>
                        {(v) => (
                            <li>
                                <span class="sysinfo-focus-volume-name">{v.mount}</span>
                                {`${formatGB(v.freeGB)} free of ${formatGB(v.totalGB)}`}
                            </li>
                        )}
                    </For>
                </ul>
            </Show>
        </>
    );
}

function NetCharts({ s }: { s: FocusState }): JSX.Element {
    return (
        <>
            {focusChart(
                s,
                () => "In above, out below",
                [
                    { key: "net:bytesrecv", label: "In", color: IN },
                    { key: "net:bytessent", label: "Out", color: OUT, below: true },
                ],
                (r) => mirroredRange(r, "net:bytesrecv", "net:bytessent", 1),
                formatRate,
                { top: true }
            )}
            {focusChart(
                s,
                () => titled("In", figures(s.rows(), "net:bytesrecv", formatRate)),
                [{ key: "net:bytesrecv", label: "In", color: IN }],
                (r) => yRange(r, "net:bytesrecv", DefaultPlotMeta["net:bytesrecv"]),
                formatRate
            )}
            {focusChart(
                s,
                () => titled("Out", figures(s.rows(), "net:bytessent", formatRate)),
                [{ key: "net:bytessent", label: "Out", color: OUT }],
                (r) => yRange(r, "net:bytessent", DefaultPlotMeta["net:bytessent"]),
                formatRate,
                { timeAxis: () => true }
            )}
        </>
    );
}

export function FocusView(props: FocusProps): JSX.Element {
    const s = createFocusState(props);

    return (
        <div class="sysinfo-focus">
            <Switch>
                <Match when={props.area === "cpu"}>
                    <CpuCharts s={s} />
                </Match>
                <Match when={props.area === "mem"}>
                    <MemCharts s={s} />
                </Match>
                <Match when={props.area === "disk"}>
                    <DiskCharts s={s} />
                </Match>
                <Match when={props.area === "net"}>
                    <NetCharts s={s} />
                </Match>
            </Switch>
        </div>
    );
}
