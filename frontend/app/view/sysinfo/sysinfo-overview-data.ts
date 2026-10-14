// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Overview's figures, as pure functions: each metric's now, low, high and
 * average over the visible window, the cores a machine reports, the per-core
 * grid behind the heatmap, the range of an in/out chart, and how rates and
 * sizes read.
 * docs/reports/REPORT_SYSINFO_REWRITE_AND_MODERN_MONITOR_2026_10_13.md section 5.3.
 */

import { niceCeil } from "./sysinfo-chart";
import type { DataItem } from "./sysinfo-types";

/** The visible window's view of one metric. */
export interface WindowStats {
    now: number;
    min: number;
    max: number;
    avg: number;
}

function reading(row: DataItem, key: string): number | null {
    if (row.blank === 1) return null;
    const v = row[key];
    return typeof v === "number" && Number.isFinite(v) ? v : null;
}

/** `key`'s newest reading and its low, high and average over `rows`; `null` with none. */
export function windowStats(rows: DataItem[], key: string): WindowStats | null {
    let now: number | null = null;
    let min = Infinity;
    let max = -Infinity;
    let sum = 0;
    let n = 0;
    for (const row of rows) {
        const v = reading(row, key);
        if (v == null) continue;
        now = v;
        min = Math.min(min, v);
        max = Math.max(max, v);
        sum += v;
        n++;
    }
    return now == null ? null : { now, min, max, avg: sum / n };
}

/** The per-core keys (`cpu:0`, `cpu:1`, ...) a sample carries, in core order. */
export function coreKeys(row: DataItem | undefined): string[] {
    if (!row) return [];
    return Object.keys(row)
        .filter((k) => /^cpu:\d+$/.test(k))
        .sort((a, b) => Number(a.slice(4)) - Number(b.slice(4)));
}

/** The heatmap's grid: one row per core, one column per sample, `null` where there is no reading. */
export function coreGrid(rows: DataItem[], cores: string[]): (number | null)[][] {
    return cores.map((key) => rows.map((row) => reading(row, key)));
}

/**
 * The range of an in/out chart: the first key drawn above the axis, the second
 * below it, both on one scale, so equal amounts look equal. Symmetric, at least
 * `floor`, rounded up to a value that reads well.
 */
export function mirroredRange(rows: DataItem[], upKey: string, downKey: string, floor: number): [number, number] {
    let peak = 0;
    for (const row of rows) {
        peak = Math.max(peak, reading(row, upKey) ?? 0, reading(row, downKey) ?? 0);
    }
    const top = niceCeil(Math.max(peak * 1.15, floor));
    return [-top, top];
}

/** A rate given in MB/s, in the unit that reads best: KB/s under 1 MB/s, GB/s from 1000. */
export function formatRate(mbps: number | null | undefined): string {
    if (mbps == null || !Number.isFinite(mbps)) return "–";
    const v = Math.abs(mbps);
    if (v >= 1000) return `${(v / 1000).toFixed(1)} GB/s`;
    if (v >= 1) return `${v.toFixed(1)} MB/s`;
    return `${(v * 1000).toFixed(0)} KB/s`;
}

/** A size given in GB, to one decimal. */
export function formatGB(gb: number | null | undefined): string {
    if (gb == null || !Number.isFinite(gb)) return "–";
    return `${gb.toFixed(1)} GB`;
}

/** The window choices, in samples at the default 1 s interval. */
export const WINDOW_CHOICES = [
    { points: 60, label: "1m" },
    { points: 120, label: "2m" },
    { points: 300, label: "5m" },
    { points: 900, label: "15m" },
] as const;
