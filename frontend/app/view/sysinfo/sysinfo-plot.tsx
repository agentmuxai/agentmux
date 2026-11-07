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
import { createEffect, createMemo, createSignal, on, onCleanup, onMount, Show, useContext } from "solid-js";
import { throttle } from "throttle-debounce";
import uPlot from "uplot";

import { appliedWindowTheme } from "@/app/store/window-appearance";

import "uplot/dist/uPlot.min.css";
import "./sysinfo-plot.scss";
import { SysinfoZoomContext } from "./sysinfo-zoom";

import {
    fadeLeftEnd,
    formatValue,
    resolveCssColor,
    type SeriesSpec,
    timeAxisLabels,
    toSeriesData,
    withAlpha,
    xWindowSecs,
    type YLabel,
    yLabelTexts,
    yRange,
} from "./sysinfo-chart";
import { axisFontPx, FADE_EXTRA_PX, minFullChartPx, TICK_GAP_PX, TICK_PX, timeAxisPx, topPaddingPx } from "./sysinfo-layout";
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
 * The time labels, with the ones that would cross either of the plot's edges left
 * blank (their ticks stay): the plot runs to both, and a label centred on a tick
 * near one would be cut off.
 */
function edgeSafe(u: uPlot, splits: number[], labels: string[]): string[] {
    const right = u.bbox.width / uPlot.pxRatio;
    if (right <= 0) return labels;
    u.ctx.font = (u.axes[0] as any).font[0];
    return labels.map((label, i) => {
        const half = u.ctx.measureText(label).width / uPlot.pxRatio / 2;
        const x = u.valToPos(splits[i], "x");
        return x + half + TEXT_INSET_PX > right || x - half - TEXT_INSET_PX < 0 ? "" : label;
    });
}

/** The widest y label's width, set on the chart for its title (sysinfo-plot.scss). */
const YLABEL_WIDTH_VAR = "--sysinfo-ylabel-w";

/** The y labels over the plot's left end, and the fade under them; none, and nothing is drawn or faded. */
function placeYLabels(u: uPlot, into: HTMLElement, labels: YLabel[]): void {
    if (labels.length === 0) {
        into.replaceChildren();
        into.parentElement?.style.removeProperty(YLABEL_WIDTH_VAR);
        return;
    }
    u.ctx.font = (u.axes[1] as any).font[0];
    const widest = Math.max(...labels.map((l) => u.ctx.measureText(l.text).width)) / uPlot.pxRatio;
    fadeLeftEnd(u, (widest + TEXT_INSET_PX + FADE_EXTRA_PX) * uPlot.pxRatio);
    // The title shares the top line with the top label: it keeps to the right of it.
    into.parentElement?.style.setProperty(YLABEL_WIDTH_VAR, `${Math.ceil(widest) + TEXT_INSET_PX}px`);
    into.replaceChildren(
        ...labels.map((l) => {
            const el = document.createElement("div");
            el.className = "sysinfo-chart-ylabel";
            el.textContent = l.text;
            el.style.top = `${u.over.offsetTop + u.valToPos(l.value, "y")}px`;
            // The lowest hangs above its line, not over the time axis.
            el.style.transform = l.lowest ? "translateY(-100%)" : "translateY(-50%)";
            return el;
        })
    );
}

/**
 * The y axis: it draws the grid and reports its ticks (`onSplits`), and has no width and no labels of its own.
 * The plot runs under the labels, which `placeYLabels` puts over it.
 */
function yAxisOptions(base: Pick<uPlot.Axis, "stroke" | "font" | "grid">, space: number, onSplits: (splits: number[]) => void): uPlot.Axis {
    return {
        ...base,
        size: 0,
        gap: 0,
        ticks: { show: false },
        space,
        values: (_u, splits) => {
            onSplits(splits);
            return splits.map(() => "");
        },
    };
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
    /** `false`: no y labels, for a chart whose row neighbour on the left has the same range. */
    yLabels?: boolean;
    /** In the pane's top row: the title keeps clear of the floating buttons. */
    top?: boolean;
    /** The chart's name and figures, centred over the top of the plot. */
    title?: string;
    class?: string;
};

/**
 * A time chart of one or more series, the ones marked `below` drawn under the
 * axis. It runs flush to the edges of its box (so to the pane's): the plot and
 * the line touch the left and right edges, and text sits TEXT_INSET_PX in. The
 * y labels are drawn over the plot's left end, which fades out under them
 * (`fadeLeftEnd`), so the series runs to the edge and the labels stay readable.
 */
export function TimeChart(props: TimeChartProps): JSX.Element {
    let container!: HTMLDivElement;
    let tip!: HTMLDivElement;
    let yAxis!: HTMLDivElement;
    let chart: uPlot | null = null;
    const [size, setSize] = createSignal({ width: 0, height: 0 });
    const zoom = useContext(SysinfoZoomContext);
    // Too short for axes at this zoom: drawn bare, title and tooltip kept (§5 C.2 rule 8).
    const bare = createMemo(
        () => !!props.sparkline || (size().height > 0 && size().height < minFullChartPx(zoom(), props.timeAxis !== false))
    );
    // The y axis' tick values, kept for the labels drawn over the plot.
    let ySplits: number[] = [];
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

    /** After uPlot has drawn the plot: the fade at its left end, then the y labels over it. */
    const drawYLabels = (u: uPlot) => {
        const labels =
            bare() || props.yLabels === false
                ? []
                : yLabelTexts(ySplits, u.scales.y.min ?? 0, u.scales.y.max ?? 1, (v) =>
                      (props.axisFormat ?? props.format)(Math.abs(v))
                  );
        placeYLabels(u, yAxis, labels);
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
            // Nothing on either side: the plot runs to both edges (a time label that would
            // cross the right one is dropped, see `edgeSafe`). The top keeps room for the top y label.
            ...(bare() ? {} : { padding: [topPaddingPx(z), 0, null, 0] as uPlot.Padding }),
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
                      yAxisOptions({ stroke: muted, font, grid }, Math.round(Y_LABEL_SPACE_PX * z), (splits) => (ySplits = splits)),
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
            hooks: { setCursor: [(u) => showTip(u)], draw: [(u) => drawYLabels(u)] },
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

    return (
        <div class={`sysinfo-chart ${props.top ? "sysinfo-chart--top" : ""} ${props.class ?? ""}`}>
            <Show when={props.title}>
                <div class="sysinfo-chart-title">{props.title}</div>
            </Show>
            <div ref={container} class="sysinfo-chart-canvas" />
            <div ref={yAxis} class="sysinfo-chart-yaxis" />
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
} & Pick<TimeChartProps, "timeAxis" | "yLabels" | "top">;

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
            class="sysinfo-grid-chart"
        />
    );
}
