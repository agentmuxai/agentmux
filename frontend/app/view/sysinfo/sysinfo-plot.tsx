// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One Sysinfo chart, drawn by uPlot on a canvas. It is built once and given new
 * data with `setData` on each update, instead of a new SVG every time, so a
 * busy pane costs a redraw, not a rebuild. What it draws (the series, window,
 * range and labels) comes from sysinfo-chart.ts.
 * docs/reports/REPORT_SYSINFO_REWRITE_AND_MODERN_MONITOR_2026_10_13.md section 5.2.
 */

import type { JSX } from "solid-js";
import { createEffect, createSignal, on, onCleanup, onMount, Show } from "solid-js";
import { throttle } from "throttle-debounce";
import uPlot from "uplot";

import { appliedWindowTheme } from "@/app/store/window-appearance";

import "uplot/dist/uPlot.min.css";
import "./sysinfo-plot.scss";

import {
    formatValue,
    resolveCssColor,
    timeAxisLabels,
    toAlignedData,
    withAlpha,
    xWindowSecs,
    yRange,
} from "./sysinfo-chart";
import type { DataItem } from "./sysinfo-types";

type SingleLinePlotProps = {
    plotData: Array<DataItem>;
    yval: string;
    yvalMeta: TimeSeriesMeta;
    blockId: string;
    defaultColor: string;
    title?: boolean;
    sparkline?: boolean;
    targetLen: number;
    intervalSecs: number;
    /** Charts with the same key move one crosshair together. */
    syncKey?: string;
};

/** At most one resize per this many ms while the pane is being dragged. */
const CHART_RESIZE_MS = 100;
/** Least room per x label, so a narrow pane gets fewer of them. */
const X_LABEL_SPACE_PX = 80;

/** Width of a y axis that fits its widest label (uPlot's own sizing recipe). */
function yAxisSize(self: uPlot, values: string[] | null, axisIdx: number, cycleNum: number): number {
    const axis = self.axes[axisIdx];
    if (cycleNum > 1) return (axis as any)._size;
    let size = (axis.ticks?.size ?? 0) + (axis.gap ?? 0);
    const longest = (values ?? []).reduce((a, v) => (v.length > a.length ? v : a), "");
    if (longest !== "") {
        self.ctx.font = (axis as any).font[0];
        size += self.ctx.measureText(longest).width / devicePixelRatio;
    }
    return Math.ceil(size);
}

function SingleLinePlot(props: SingleLinePlotProps): JSX.Element {
    let container!: HTMLDivElement;
    let tip!: HTMLDivElement;
    let chart: uPlot | null = null;
    const [size, setSize] = createSignal({ width: 0, height: 0 });
    // What the scales read; updated before each setData.
    let xWindow: [number, number] = [0, 1];
    let yWindow: [number, number] = [0, 100];

    const unit = () => props.yvalMeta?.label ?? "";
    const decimals = () => props.yvalMeta?.decimalPlaces ?? 0;

    const showTip = (u: uPlot) => {
        const idx = u.cursor.idx;
        const y = idx == null ? null : (u.data[1][idx] as number | null);
        if (idx == null || y == null || props.sparkline) {
            tip.style.display = "none";
            return;
        }
        const ts = (u.data[0][idx] as number) * 1000;
        tip.textContent = `${new Date(ts).toLocaleTimeString()}  ${formatValue(y, decimals(), unit())}`;
        tip.style.display = "block";
        tip.style.left = `${u.over.offsetLeft + (u.cursor.left ?? 0)}px`;
        tip.style.top = `${Math.max(0, u.over.offsetTop + u.valToPos(y, "y") - 30)}px`;
    };

    const options = (width: number, height: number): uPlot.Options => {
        const color = resolveCssColor(container, props.yvalMeta?.color || props.defaultColor);
        const muted = resolveCssColor(container, "var(--grey-text-color)");
        const grid = { stroke: withAlpha(muted, 0.25), width: 1 };
        const ticks = { stroke: withAlpha(muted, 0.4), width: 1, size: 4 };
        return {
            width,
            height,
            legend: { show: false },
            cursor: {
                // Only the time is shared: each chart has its own y scale, and by
                // default uPlot would also draw the hovered chart's y value on the others.
                sync: props.syncKey ? { key: props.syncKey, scales: ["x", null] } : undefined,
                drag: { x: false, y: false },
                points: { size: 6, fill: color },
            },
            scales: {
                x: { time: true, range: () => xWindow },
                y: { range: () => yWindow },
            },
            axes: props.sparkline
                ? [{ show: false }, { show: false }]
                : [
                      {
                          stroke: muted,
                          grid,
                          ticks,
                          space: X_LABEL_SPACE_PX,
                          values: (_u, splits, _axisIdx, _space, incr) => timeAxisLabels(splits, incr),
                      },
                      {
                          stroke: muted,
                          grid,
                          ticks,
                          size: yAxisSize,
                          values: (_u, splits) => splits.map((v) => formatValue(v, decimals(), "")),
                      },
                  ],
            series: [
                {},
                {
                    label: props.yvalMeta?.name ?? props.yval,
                    stroke: color,
                    width: 2,
                    // A missing sample is a break in the line and the area.
                    spanGaps: false,
                    points: { show: false },
                    fill: (u: uPlot) => {
                        const g = u.ctx.createLinearGradient(0, u.bbox.top, 0, u.bbox.top + u.bbox.height);
                        g.addColorStop(0, withAlpha(color, 0.7));
                        g.addColorStop(1, withAlpha(color, 0));
                        return g;
                    },
                },
            ],
            hooks: { setCursor: [(u) => showTip(u)] },
        };
    };

    const windows = () => {
        const rows = props.plotData ?? [];
        const win = xWindowSecs(rows, props.targetLen, props.intervalSecs);
        if (win) xWindow = win;
        yWindow = yRange(rows, props.yval, props.yvalMeta);
        return toAlignedData(rows, [props.yval]);
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

    // A new chart when what it draws changes (metric, color, axes, or the
    // theme, whose colors the canvas resolved when it was built), or when it
    // has none yet; otherwise only a new size, or new data.
    createEffect(
        on(
            [
                () => props.yval,
                () => props.yvalMeta,
                () => props.defaultColor,
                () => props.sparkline,
                appliedWindowTheme,
            ],
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
        <div class="sysinfo-chart min-h-[100px]">
            <Show when={props.title && props.yvalMeta?.name}>
                <div class="sysinfo-chart-title">{props.yvalMeta?.name}</div>
            </Show>
            <div ref={container} class="sysinfo-chart-canvas" />
            <div ref={tip} class="sysinfo-chart-tip" style={{ display: "none" }} />
        </div>
    );
}

export { SingleLinePlot };
