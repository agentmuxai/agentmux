// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Sysinfo's charts, drawn by uPlot on a canvas. A chart is built once and given
 * new data with `setData` on each update, instead of a new SVG every time, so a
 * busy pane costs a redraw, not a rebuild. What it draws (the series, window,
 * range and labels) comes from sysinfo-chart.ts.
 * docs/reports/REPORT_SYSINFO_REWRITE_AND_MODERN_MONITOR_2026_10_13.md section 5.2.
 */

import type { JSX } from "solid-js";
import { createContext, createEffect, createMemo, createSignal, on, onCleanup, onMount, Show, useContext } from "solid-js";
import { throttle } from "throttle-debounce";
import uPlot from "uplot";

import { appliedWindowTheme } from "@/app/store/window-appearance";

import "uplot/dist/uPlot.min.css";
import "./sysinfo-plot.scss";

import {
    formatValue,
    resolveCssColor,
    type SeriesSpec,
    timeAxisLabels,
    toSeriesData,
    withAlpha,
    xWindowSecs,
    yRange,
} from "./sysinfo-chart";
import { axisFontPx, BARE_AXIS_PX, minFullChartPx, TICK_GAP_PX, TICK_PX, timeAxisPx, topPaddingPx } from "./sysinfo-layout";
import { figures, titled, unitFormat } from "./sysinfo-overview-data";
import type { DataItem } from "./sysinfo-types";

/** At most one resize per this many ms while the pane is being dragged. */
const CHART_RESIZE_MS = 100;
/** Least room per x label, so a narrow pane gets fewer of them. */
const X_LABEL_SPACE_PX = 80;

/** Text keeps this far from the pane's edges; graphics (the plot, the line) touch them. */
const TEXT_INSET_PX = 1;
/** Least room between y ticks at 100% zoom (uPlot's default), so a short chart shows fewer labels. */
const Y_LABEL_SPACE_PX = 30;

/**
 * The pane's zoom (`term:zoom`) for Sysinfo's text: the charts' axis labels and
 * the room they take scale with it, their lines and fills don't. Provided by the
 * view (sysinfo-view.tsx); 1 outside it.
 */
export const SysinfoZoomContext = createContext<() => number>(() => 1);

/**
 * Width of a y axis that fits its widest label (uPlot's own sizing recipe). It
 * reports that width (`report`) and takes at least `min()`: its column's widest,
 * so the plots in a column line up.
 */
const yAxisSize =
    (min: () => number, report: (w: number) => void) =>
    (self: uPlot, values: string[] | null, axisIdx: number, cycleNum: number) => {
        const axis = self.axes[axisIdx];
        if (cycleNum > 1) return (axis as any)._size;
        let size = (axis.ticks?.size ?? 0) + (axis.gap ?? 0);
        const longest = (values ?? []).reduce((a, v) => (v.length > a.length ? v : a), "");
        if (longest !== "") {
            self.ctx.font = (axis as any).font[0];
            size += self.ctx.measureText(longest).width / devicePixelRatio;
        }
        const natural = Math.ceil(size);
        report(natural);
        return Math.max(min(), natural);
    };

/**
 * The time labels, with the ones that would cross the plot's right edge left
 * blank (their ticks stay): the plot runs to the edge, and a label centred on a
 * tick near it would be cut off.
 */
function edgeSafe(u: uPlot, splits: number[], labels: string[]): string[] {
    const right = u.bbox.width / uPlot.pxRatio;
    if (right <= 0) return labels;
    u.ctx.font = (u.axes[0] as any).font[0];
    return labels.map((label, i) => {
        const half = u.ctx.measureText(label).width / uPlot.pxRatio / 2;
        return u.valToPos(splits[i], "x") + half + TEXT_INSET_PX > right ? "" : label;
    });
}

export type TimeChartProps = {
    plotData: Array<DataItem>;
    series: SeriesSpec[];
    /** The y range for the rows on screen. */
    yRange: (rows: DataItem[]) => [number, number];
    /** How a value reads in the tooltip, and on the y axis unless `axisFormat` is given. */
    format: (v: number) => string;
    /** How a value reads on the y axis, if not as in the tooltip. */
    axisFormat?: (v: number) => string;
    targetLen: number;
    intervalSecs: number;
    /** Charts with the same key move one crosshair together. */
    syncKey?: string;
    /** No axes and no tooltip: a tile's sparkline. */
    sparkline?: boolean;
    /** `false`: no time labels, for a chart stacked above another that has them. */
    timeAxis?: boolean;
    /** `false`: tick marks but no y labels, for a chart whose row neighbour on the left has the same range. */
    yLabels?: boolean;
    /** Least y-axis width: its column's widest, so the plots line up. */
    axisWidth?: number;
    /** The y-axis width this chart's own labels need. */
    onAxisWidth?: (width: number) => void;
    /** In the pane's top row: the title keeps clear of the floating buttons. */
    top?: boolean;
    /** The chart's name and figures, centred over the top of the plot. */
    title?: string;
    class?: string;
};

/**
 * A time chart of one or more series, the ones marked `below` drawn under the
 * axis. It runs flush to the edges of its box (so to the pane's): the plot and
 * the line touch the right edge, axes are only as wide as their labels, and text
 * sits TEXT_INSET_PX in.
 */
export function TimeChart(props: TimeChartProps): JSX.Element {
    let container!: HTMLDivElement;
    let tip!: HTMLDivElement;
    let chart: uPlot | null = null;
    const [size, setSize] = createSignal({ width: 0, height: 0 });
    const zoom = useContext(SysinfoZoomContext);
    // Too short for axes at this zoom: drawn bare, title and tooltip kept (§5 C.2 rule 8).
    const bare = createMemo(
        () => !!props.sparkline || (size().height > 0 && size().height < minFullChartPx(zoom(), props.timeAxis !== false))
    );
    // What the scales read; updated before each setData.
    let xWindow: [number, number] = [0, 1];
    let yWindow: [number, number] = [0, 100];

    const showTip = (u: uPlot) => {
        const idx = u.cursor.idx;
        if (props.sparkline || idx == null) {
            tip.style.display = "none";
            return;
        }
        const parts: string[] = [];
        let anchor: number | null = null;
        props.series.forEach((s, i) => {
            const v = u.data[i + 1][idx] as number | null;
            if (v == null) return;
            anchor ??= v;
            parts.push(props.series.length > 1 ? `${s.label} ${props.format(Math.abs(v))}` : props.format(Math.abs(v)));
        });
        if (anchor == null) {
            tip.style.display = "none";
            return;
        }
        const ts = (u.data[0][idx] as number) * 1000;
        tip.textContent = `${new Date(ts).toLocaleTimeString()}  ${parts.join("  ")}`;
        tip.style.display = "block";
        // Centred on the cursor, kept inside the chart so the newest samples (at the edge) stay readable.
        const half = tip.offsetWidth / 2;
        const x = u.over.offsetLeft + (u.cursor.left ?? 0);
        tip.style.left = `${Math.max(half, Math.min(x, container.clientWidth - half))}px`;
        tip.style.top = `${Math.max(0, u.over.offsetTop + u.valToPos(anchor, "y") - 30)}px`;
    };

    const options = (width: number, height: number): uPlot.Options => {
        const muted = resolveCssColor(container, "var(--grey-text-color)");
        const grid = { stroke: withAlpha(muted, 0.25), width: 1 };
        const ticks = { stroke: withAlpha(muted, 0.4), width: 1, size: TICK_PX };
        const gap = TICK_GAP_PX;
        const single = props.series.length === 1;
        const z = zoom();
        const font = `${axisFontPx(z)}px ${getComputedStyle(container).fontFamily || "sans-serif"}`;
        const timeLabels = props.timeAxis !== false;
        return {
            width,
            height,
            legend: { show: false },
            // Nothing on the right: the plot ends at the edge (a label that would
            // cross it is dropped, see `edgeSafe`). The top keeps room for the top y label.
            ...(bare() ? {} : { padding: [topPaddingPx(z), 0, null, TEXT_INSET_PX] as uPlot.Padding }),
            cursor: {
                // Only the time is shared: each chart has its own y scale, and by
                // default uPlot would also draw the hovered chart's y value on the others.
                sync: props.syncKey ? { key: props.syncKey, scales: ["x", null] } : undefined,
                drag: { x: false, y: false },
                points: { size: 6 },
            },
            scales: {
                x: { time: true, range: () => xWindow },
                y: { range: () => yWindow },
            },
            axes: bare()
                ? [{ show: false }, { show: false }]
                : [
                      {
                          stroke: muted,
                          font,
                          grid,
                          ticks,
                          gap,
                          space: Math.round(X_LABEL_SPACE_PX * z),
                          values: (u, splits, _axisIdx, _space, incr) =>
                              timeLabels ? edgeSafe(u, splits, timeAxisLabels(splits, incr)) : splits.map(() => ""),
                          size: timeAxisPx(z, timeLabels),
                      },
                      {
                          stroke: muted,
                          font,
                          grid,
                          ticks,
                          gap,
                          space: Math.round(Y_LABEL_SPACE_PX * z),
                          size:
                              props.yLabels === false
                                  ? BARE_AXIS_PX
                                  : yAxisSize(
                                        () => props.axisWidth ?? 0,
                                        (w) => queueMicrotask(() => props.onAxisWidth?.(w))
                                    ),
                          // Below the axis is still an amount: no minus signs.
                          values: (_u, splits) =>
                              props.yLabels === false
                                  ? splits.map(() => "")
                                  : splits.map((v) => (props.axisFormat ?? props.format)(Math.abs(v))),
                      },
                  ],
            series: [
                {},
                ...props.series.map((s) => {
                    const color = resolveCssColor(container, s.color);
                    return {
                        label: s.label,
                        stroke: color,
                        width: props.sparkline ? 1.5 : 2,
                        // A missing sample is a break in the line and the area.
                        spanGaps: false,
                        points: { show: false },
                        fillTo: 0,
                        fill: single
                            ? (u: uPlot) => {
                                  const g = u.ctx.createLinearGradient(0, u.bbox.top, 0, u.bbox.top + u.bbox.height);
                                  g.addColorStop(0, withAlpha(color, 0.7));
                                  g.addColorStop(1, withAlpha(color, 0));
                                  return g;
                              }
                            : withAlpha(color, 0.25),
                    } as uPlot.Series;
                }),
            ],
            hooks: { setCursor: [(u) => showTip(u)] },
        };
    };

    const windows = () => {
        const rows = props.plotData ?? [];
        const win = xWindowSecs(rows, props.targetLen, props.intervalSecs);
        if (win) xWindow = win;
        yWindow = props.yRange(rows);
        return toSeriesData(rows, props.series);
    };

    const build = () => {
        chart?.destroy();
        chart = null;
        const { width, height } = size();
        if (width <= 0 || height <= 0) return;
        chart = new uPlot(options(width, height), windows(), container);
        // No y labels to measure: say so, so an old width doesn't keep the column wide.
        if (bare() || props.yLabels === false) queueMicrotask(() => props.onAxisWidth?.(0));
    };

    onMount(() => {
        const resize = throttle(CHART_RESIZE_MS, (width: number, height: number) => setSize({ width, height }));
        const observer = new ResizeObserver((entries) => {
            const rect = entries[entries.length - 1].contentRect;
            resize(Math.floor(rect.width), Math.floor(rect.height));
        });
        observer.observe(container);
        onCleanup(() => {
            observer.disconnect();
            resize.cancel();
            chart?.destroy();
            chart = null;
        });
    });

    // A new chart when what it draws changes (series, colors, axes, or the
    // theme, whose colors the canvas resolved when it was built), or when it
    // has none yet; otherwise only a new size, or new data.
    createEffect(
        on(
            [() => props.series, bare, () => props.timeAxis, () => props.yLabels, zoom, appliedWindowTheme],
            () => build(),
            { defer: true }
        )
    );
    createEffect(
        on(size, (s) => {
            if (!chart) build();
            else chart.setSize(s);
        })
    );
    createEffect(
        on([() => props.plotData, () => props.targetLen, () => props.intervalSecs], () => chart?.setData(windows()), {
            defer: true,
        })
    );
    // The column's y-axis width changed: lay the axes out again, keeping the paths.
    createEffect(on(() => props.axisWidth, () => chart?.redraw(false, true), { defer: true }));

    return (
        <div class={`sysinfo-chart ${props.top ? "sysinfo-chart--top" : ""} ${props.class ?? ""}`}>
            <Show when={props.title}>
                <div class="sysinfo-chart-title">{props.title}</div>
            </Show>
            <div ref={container} class="sysinfo-chart-canvas" />
            <div ref={tip} class="sysinfo-chart-tip" style={{ display: "none" }} />
        </div>
    );
}

type SingleLinePlotProps = {
    plotData: Array<DataItem>;
    yval: string;
    yvalMeta: TimeSeriesMeta;
    blockId: string;
    defaultColor: string;
    sparkline?: boolean;
    targetLen: number;
    intervalSecs: number;
    syncKey?: string;
} & Pick<TimeChartProps, "timeAxis" | "yLabels" | "top" | "axisWidth" | "onAxisWidth">;

/**
 * One metric, as a simple view shows it: titled with its figures now and over
 * the window, its unit on the axis, its color and range from its metadata.
 */
export function SingleLinePlot(props: SingleLinePlotProps): JSX.Element {
    const format = (v: number) => unitFormat(props.yvalMeta)(v);
    const name = () => props.yvalMeta?.name ?? props.yval;
    const series = (): SeriesSpec[] => [
        {
            key: props.yval,
            label: name(),
            color: props.yvalMeta?.color || props.defaultColor,
        },
    ];
    return (
        <TimeChart
            plotData={props.plotData}
            series={series()}
            yRange={(rows) => yRange(rows, props.yval, props.yvalMeta)}
            format={format}
            targetLen={props.targetLen}
            intervalSecs={props.intervalSecs}
            syncKey={props.syncKey}
            sparkline={props.sparkline}
            title={props.sparkline ? undefined : titled(name(), figures(props.plotData, props.yval, format))}
            timeAxis={props.timeAxis}
            yLabels={props.yLabels}
            top={props.top}
            axisWidth={props.axisWidth}
            onAxisWidth={props.onAxisWidth}
            class="sysinfo-grid-chart"
        />
    );
}
