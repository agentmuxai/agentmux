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
    type FocusArea,
    formatGB,
    formatRate,
    mirroredRange,
    volumes,
    windowStats,
    withBusiestCore,
} from "./sysinfo-overview-data";
import { TimeChart } from "./sysinfo-plot";
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

/** "29% now, 16% – 62%": a chart's title figures over the window. */
function figures(rows: DataItem[], key: string, f: (v: number) => string): string {
    const s = windowStats(rows, key);
    return s ? `${f(s.now)} now, ${f(s.min)} – ${f(s.max)}` : "";
}

const titled = (name: string, fig: string) => (fig ? `${name}: ${fig}` : name);

export function FocusView(props: FocusProps): JSX.Element {
    const rows = () => props.plotData;
    const latest = () => rows()[rows().length - 1];
    const cores = createMemo(() => coreKeys(latest()));
    const cpuRows = createMemo(() => (props.area === "cpu" ? withBusiestCore(rows(), cores()) : rows()));
    const hasCommit = () => latest()?.["mem:commit:total"] > 0;
    // Committed memory is read against its limit, so its scale ends there.
    const commitMeta = { ...DefaultPlotMeta["mem:used"], maxy: "mem:commit:total", autoMaxY: false } as TimeSeriesMeta;

    // The title is read inside the chart, so new samples change its text, not the chart.
    const chart = (
        title: () => string,
        series: SeriesSpec[],
        range: (r: DataItem[]) => [number, number],
        format: (v: number) => string,
        opts: { data?: () => DataItem[]; timeAxis?: () => boolean } = {}
    ) => (
        <TimeChart
            plotData={(opts.data ?? rows)()}
            series={series}
            yRange={range}
            format={format}
            targetLen={props.targetLen}
            intervalSecs={props.intervalSecs}
            syncKey={props.syncKey}
            title={title()}
            timeAxis={opts.timeAxis?.() ?? false}
            class="sysinfo-focus-chart"
        />
    );

    return (
        <div class="sysinfo-focus">
            <Switch>
                <Match when={props.area === "cpu"}>
                    {chart(
                        () => titled("CPU", figures(rows(), "cpu", pct)),
                        [{ key: "cpu", label: "CPU", color: CPU }],
                        () => [0, 100],
                        pct,
                        { timeAxis: () => cores().length === 0 }
                    )}
                    <Show when={cores().length > 0}>
                        {chart(
                            () => titled("Busiest core", figures(cpuRows(), BUSIEST_CORE, pct)),
                            [
                                { key: BUSIEST_CORE, label: "Busiest core", color: OUT },
                                { key: "cpu", label: "Average", color: CPU },
                            ],
                            () => [0, 100],
                            pct,
                            { data: cpuRows, timeAxis: () => true }
                        )}
                        <div class="sysinfo-focus-chart sysinfo-overview-cores">
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
                </Match>
                <Match when={props.area === "mem"}>
                    {chart(
                        () => titled("Used", figures(rows(), "mem:used", formatGB)),
                        [{ key: "mem:used", label: "Used", color: MEM }],
                        (r) => yRange(r, "mem:used", DefaultPlotMeta["mem:used"]),
                        formatGB
                    )}
                    {chart(
                        () => titled("Available", figures(rows(), "mem:available", formatGB)),
                        [{ key: "mem:available", label: "Available", color: MEM }],
                        (r) => yRange(r, "mem:available", DefaultPlotMeta["mem:used"]),
                        formatGB,
                        { timeAxis: () => !hasCommit() }
                    )}
                    <Show when={hasCommit()}>
                        {chart(
                            () => titled("Committed", figures(rows(), "mem:commit:used", formatGB)),
                            [{ key: "mem:commit:used", label: "Committed", color: OUT }],
                            (r) => yRange(r, "mem:commit:used", commitMeta),
                            formatGB,
                            { timeAxis: () => true }
                        )}
                    </Show>
                </Match>
                <Match when={props.area === "disk"}>
                    {chart(
                        () => "Read above, write below",
                        [
                            { key: "disk:read", label: "Read", color: IN },
                            { key: "disk:write", label: "Write", color: OUT, below: true },
                        ],
                        (r) => mirroredRange(r, "disk:read", "disk:write", 1),
                        formatRate
                    )}
                    {chart(
                        () => titled("Read", figures(rows(), "disk:read", formatRate)),
                        [{ key: "disk:read", label: "Read", color: IN }],
                        (r) => yRange(r, "disk:read", DefaultPlotMeta["disk:read"]),
                        formatRate
                    )}
                    {chart(
                        () => titled("Write", figures(rows(), "disk:write", formatRate)),
                        [{ key: "disk:write", label: "Write", color: OUT }],
                        (r) => yRange(r, "disk:write", DefaultPlotMeta["disk:write"]),
                        formatRate,
                        { timeAxis: () => true }
                    )}
                    <Show when={volumes(latest()).length > 0}>
                        <ul class="sysinfo-focus-volumes" aria-label="Volumes">
                            <For each={volumes(latest())}>
                                {(v) => (
                                    <li>
                                        <span class="sysinfo-focus-volume-name">{v.mount}</span>
                                        {`${formatGB(v.freeGB)} free of ${formatGB(v.totalGB)}`}
                                    </li>
                                )}
                            </For>
                        </ul>
                    </Show>
                </Match>
                <Match when={props.area === "net"}>
                    {chart(
                        () => "In above, out below",
                        [
                            { key: "net:bytesrecv", label: "In", color: IN },
                            { key: "net:bytessent", label: "Out", color: OUT, below: true },
                        ],
                        (r) => mirroredRange(r, "net:bytesrecv", "net:bytessent", 1),
                        formatRate
                    )}
                    {chart(
                        () => titled("In", figures(rows(), "net:bytesrecv", formatRate)),
                        [{ key: "net:bytesrecv", label: "In", color: IN }],
                        (r) => yRange(r, "net:bytesrecv", DefaultPlotMeta["net:bytesrecv"]),
                        formatRate
                    )}
                    {chart(
                        () => titled("Out", figures(rows(), "net:bytessent", formatRate)),
                        [{ key: "net:bytessent", label: "Out", color: OUT }],
                        (r) => yRange(r, "net:bytessent", DefaultPlotMeta["net:bytessent"]),
                        formatRate,
                        { timeAxis: () => true }
                    )}
                </Match>
            </Switch>
        </div>
    );
}
