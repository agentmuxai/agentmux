// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Every core's load over the window, one row per core and one column per
 * sample, in one hue: the stronger, the busier. A gap is left empty. It fits any
 * number of cores, where one line per core stops being readable past a few.
 * Columns sit by time on the charts' window, inside the same side gutters
 * (`PLOT_LEFT_PX`), so they line up under the charts above.
 * docs/reports/REPORT_SYSINFO_REWRITE_AND_MODERN_MONITOR_2026_10_13.md section 5.3.
 */

import type { JSX } from "solid-js";
import { createEffect, createSignal, onCleanup, onMount } from "solid-js";

import { appliedWindowTheme } from "@/app/store/window-appearance";

import { PLOT_LEFT_PX, PLOT_RIGHT_PX, resolveCssColor, xWindowSecs } from "./sysinfo-chart";
import { columnSpan, coreGrid } from "./sysinfo-overview-data";
import type { DataItem } from "./sysinfo-types";

/** The faintest a busy-at-all cell is drawn, so 1 % still shows against the background. */
const MIN_ALPHA = 0.08;

/** The alpha a load in percent is drawn with. */
function cellAlpha(pct: number): number {
    const v = Math.max(0, Math.min(100, pct)) / 100;
    return MIN_ALPHA + (1 - MIN_ALPHA) * v;
}

type CoreHeatmapProps = {
    plotData: DataItem[];
    cores: string[];
    color: string;
    /** The window, as the charts take it (`xWindowSecs`). */
    targetLen: number;
    intervalSecs: number;
};

export function CoreHeatmap(props: CoreHeatmapProps): JSX.Element {
    let host!: HTMLDivElement;
    let canvas!: HTMLCanvasElement;
    let tip!: HTMLDivElement;
    const [size, setSize] = createSignal({ width: 0, height: 0 });

    onMount(() => {
        const observer = new ResizeObserver((entries) => {
            const r = entries[entries.length - 1].contentRect;
            setSize({ width: Math.floor(r.width), height: Math.floor(r.height) });
        });
        observer.observe(host);
        onCleanup(() => observer.disconnect());
    });

    const plotWidth = (width: number) => width - PLOT_LEFT_PX - PLOT_RIGHT_PX;
    const win = (rows: DataItem[]) => xWindowSecs(rows, props.targetLen, props.intervalSecs);

    createEffect(() => {
        // Drawn again on a theme change too, even while paused: the color is read from it.
        appliedWindowTheme();
        const { width, height } = size();
        const rows = props.plotData ?? [];
        const grid = coreGrid(rows, props.cores);
        const ctx = canvas.getContext("2d");
        if (!ctx || width <= 0 || height <= 0) return;
        const dpr = window.devicePixelRatio || 1;
        canvas.width = width * dpr;
        canvas.height = height * dpr;
        canvas.style.width = `${width}px`;
        canvas.style.height = `${height}px`;
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        ctx.clearRect(0, 0, width, height);
        const w = win(rows);
        const pw = plotWidth(width);
        if (!w || pw <= 0 || grid.length === 0) return;
        // One color for every cell, its opacity set per cell: no color string per cell.
        ctx.fillStyle = resolveCssColor(host, props.color);
        const rh = height / grid.length;
        const spans = rows.map((row) => columnSpan(row.ts, w, props.intervalSecs, pw));
        grid.forEach((cells, r) => {
            cells.forEach((v, c) => {
                const span = spans[c];
                if (v == null || !span) return;
                ctx.globalAlpha = cellAlpha(v);
                // A hairline between rows keeps cores apart without a grid.
                ctx.fillRect(PLOT_LEFT_PX + span[0], r * rh, Math.ceil(span[1] - span[0]), Math.max(1, rh - 1));
            });
        });
        ctx.globalAlpha = 1;
    });

    const onMove = (e: MouseEvent) => {
        const rows = props.plotData ?? [];
        const { width, height } = size();
        const w = win(rows);
        const pw = plotWidth(width);
        if (!w || !props.cores.length || pw <= 0) return;
        const rect = host.getBoundingClientRect();
        // The sample whose column is under the pointer.
        const x = e.clientX - rect.left - PLOT_LEFT_PX;
        const c = rows.findIndex((row) => {
            const span = columnSpan(row.ts, w, props.intervalSecs, pw);
            return span != null && x >= span[0] && x < span[1];
        });
        if (c < 0) {
            tip.style.display = "none";
            return;
        }
        const r = Math.min(
            props.cores.length - 1,
            Math.max(0, Math.floor(((e.clientY - rect.top) / height) * props.cores.length))
        );
        const row = rows[c];
        const v = row.blank === 1 ? null : row[props.cores[r]];
        if (typeof v !== "number" || !Number.isFinite(v)) {
            tip.style.display = "none";
            return;
        }
        tip.textContent = `Core ${props.cores[r].slice(4)}  ${new Date(row.ts).toLocaleTimeString()}  ${v.toFixed(0)}%`;
        tip.style.display = "block";
        tip.style.left = `${e.clientX - rect.left}px`;
        tip.style.top = `${Math.max(0, e.clientY - rect.top - 30)}px`;
    };

    return (
        <div
            ref={host}
            class="sysinfo-heatmap"
            role="img"
            aria-label={`Load of ${props.cores.length} cores over the window`}
            onMouseMove={onMove}
            onMouseLeave={() => (tip.style.display = "none")}
        >
            <canvas ref={canvas} />
            <div ref={tip} class="sysinfo-chart-tip" style={{ display: "none" }} />
        </div>
    );
}
