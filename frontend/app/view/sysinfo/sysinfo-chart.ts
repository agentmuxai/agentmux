// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a Sysinfo chart draws, as pure functions: the series in uPlot's shape,
 * the visible window, the y range and the labels. The chart itself
 * (sysinfo-plot.tsx) only hands these to uPlot, so everything that decides what
 * a reader sees is tested here.
 * docs/reports/REPORT_SYSINFO_REWRITE_AND_MODERN_MONITOR_2026_10_13.md section 5.2.
 */

import type { AlignedData } from "uplot";

import type { DataItem } from "./sysinfo-types";
import { computeAutoMaxY, formatTimeTick, resolveDomainBound } from "./sysinfo-util";

/**
 * uPlot's data for `keys` over `rows`: the x values in seconds, then one series
 * per key. A gap marker, or a value that isn't a finite number, is `null`, which
 * uPlot draws as a break (`spanGaps: false`), never as a value.
 */
export function toAlignedData(rows: DataItem[], keys: string[]): AlignedData {
    const xs = rows.map((r) => r.ts / 1000);
    const ys = keys.map((k) =>
        rows.map((r) => {
            if (r.blank === 1) return null;
            const v = r[k];
            return typeof v === "number" && Number.isFinite(v) ? v : null;
        })
    );
    return [xs, ...ys] as AlignedData;
}

/** One series of a chart: which metric, its name and color, and whether it is drawn below the axis. */
export interface SeriesSpec {
    key: string;
    label: string;
    color: string;
    /** Drawn below the axis (sent, written), opposite the series above it. */
    below?: boolean;
}

/** uPlot's data for `specs`: as `toAlignedData`, with a series drawn below the axis negated. */
export function toSeriesData(rows: DataItem[], specs: SeriesSpec[]): AlignedData {
    const [xs, ...ys] = toAlignedData(
        rows,
        specs.map((s) => s.key)
    );
    return [
        xs,
        ...ys.map((y, i) => (specs[i].below ? (y as (number | null)[]).map((v) => (v == null ? null : -v)) : y)),
    ] as AlignedData;
}

/** The visible window in seconds: `targetLen` intervals ending at the newest sample; `null` with no data. */
export function xWindowSecs(rows: DataItem[], targetLen: number, intervalSecs: number): [number, number] | null {
    if (rows.length === 0) return null;
    const maxMs = rows[rows.length - 1].ts;
    return [(maxMs - targetLen * intervalSecs * 1000) / 1000, maxMs / 1000];
}

/**
 * Where a chart's plot area sits in its pane: at least this far in from the
 * left (its y axis) and this far in from the right (room for the last time
 * label). Fixed, so charts stacked in one pane and the core heatmap under them
 * share one time scale on screen.
 */
export const PLOT_LEFT_PX = 76;
export const PLOT_RIGHT_PX = 36;

/** The smallest of 1, 2, 2.5 or 5 times a power of ten at or above `v`: a ceiling that reads well on an axis. */
export function niceCeil(v: number): number {
    if (!(v > 0) || !Number.isFinite(v)) return 1;
    const pow = Math.pow(10, Math.floor(Math.log10(v)));
    for (const m of [1, 2, 2.5, 5, 10]) {
        if (m * pow >= v - pow * 1e-9) return m * pow;
    }
    return 10 * pow;
}

/**
 * The y range for `yval`:
 * - a percentage keeps its fixed 0 to 100, since auto-scaling it would make noise
 *   look dramatic;
 * - memory scales to what is used but never past its real total (`maxy`);
 * - a rate with no ceiling (network, disk) scales to the window's peak, rounded up
 *   to a value that reads well.
 * See computeAutoMaxY and REPORT_SYSINFO_COMBINED_CHART_RESEARCH_2026_08_17.md.
 */
export function yRange(rows: DataItem[], yval: string, meta: TimeSeriesMeta | undefined): [number, number] {
    const latest = rows[rows.length - 1];
    const hardCap = resolveDomainBound(meta?.maxy, latest);
    let max = meta?.autoMaxY ? computeAutoMaxY(rows, yval, meta?.autoMaxYFloor ?? 1, hardCap) : (hardCap ?? 100);
    if (meta?.autoMaxY && hardCap == null) max = niceCeil(max);
    const min = resolveDomainBound(meta?.miny, latest) ?? 0;
    return [min, max > min ? max : min + 1];
}

/** The time axis' labels, from uPlot's tick times and spacing (both in seconds). */
export function timeAxisLabels(splitsSecs: number[], incrSecs: number): string[] {
    return splitsSecs.map((s) => formatTimeTick(s * 1000, incrSecs * 1000));
}

/** A value as the y axis and the tooltip show it: rounded to the metric's places, with its unit. */
export function formatValue(v: number | null | undefined, decimals: number, unit: string): string {
    if (v == null || !Number.isFinite(v)) return "–";
    return `${v.toFixed(decimals)}${unit}`;
}

/**
 * `color` with an alpha, for the area's gradient on a canvas, which can't read a
 * CSS variable. Takes `#rgb`, `#rrggbb`, `rgb()` and `rgba()`; anything else is
 * returned as it is.
 */
export function withAlpha(color: string, alpha: number): string {
    const c = color.trim();
    const hex = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(c);
    if (hex) {
        const h = hex[1].length === 3 ? [...hex[1]].map((d) => d + d).join("") : hex[1];
        const [r, g, b] = [0, 2, 4].map((i) => parseInt(h.slice(i, i + 2), 16));
        return `rgba(${r}, ${g}, ${b}, ${alpha})`;
    }
    const rgb = /^rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)/i.exec(c);
    if (rgb) return `rgba(${rgb[1]}, ${rgb[2]}, ${rgb[3]}, ${alpha})`;
    return c;
}

/** A color a canvas can use: a `var(--x)` read from `el`'s computed style, anything else as it is. */
export function resolveCssColor(el: Element, color: string): string {
    const m = /^var\(\s*(--[\w-]+)\s*(?:,\s*([^)]+))?\)$/.exec(color.trim());
    if (!m) return color;
    const value = getComputedStyle(el).getPropertyValue(m[1]).trim();
    return value || m[2]?.trim() || color;
}
