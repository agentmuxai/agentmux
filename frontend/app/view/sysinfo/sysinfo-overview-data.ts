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

/** One area shown alone, in a pane too small for the whole Overview. */
export type FocusArea = "cpu" | "mem" | "disk" | "net";

/** The area choices; `overview` is every area at once. */
export const FOCUS_CHOICES = [
    { value: "overview", label: "Overview" },
    { value: "cpu", label: "CPU" },
    { value: "mem", label: "Memory" },
    { value: "disk", label: "Disk" },
    { value: "net", label: "Network" },
] as const;

/** A saved area, or `null` (the Overview) for anything else. */
export function toFocusArea(v: unknown): FocusArea | null {
    return v === "cpu" || v === "mem" || v === "disk" || v === "net" ? v : null;
}

/** The key `withBusiestCore` adds: the busiest core's reading in each sample. */
export const BUSIEST_CORE = "cpu:busiest";

/** `rows` with each sample's busiest core added, which shows one thread maxing out a core while the average stays low. */
export function withBusiestCore(rows: DataItem[], cores: string[]): DataItem[] {
    if (cores.length === 0) return rows;
    return rows.map((row) => {
        if (row.blank === 1) return { ...row, [BUSIEST_CORE]: NaN };
        let busiest = -Infinity;
        for (const k of cores) busiest = Math.max(busiest, reading(row, k) ?? -Infinity);
        return { ...row, [BUSIEST_CORE]: Number.isFinite(busiest) ? busiest : NaN };
    });
}

/** One mounted volume's space, from the newest sample. */
interface Volume {
    mount: string;
    freeGB: number;
    totalGB: number;
}

/** The volumes a sample carries (`disk:vol:<mount>:free_gb` and `:total_gb`), in mount order. */
export function volumes(row: DataItem | undefined): Volume[] {
    if (!row) return [];
    const out: Volume[] = [];
    for (const key of Object.keys(row)) {
        const m = /^disk:vol:(.+):free_gb$/.exec(key);
        if (!m) continue;
        const total = reading(row, `disk:vol:${m[1]}:total_gb`);
        const free = reading(row, key);
        if (total == null || free == null || total <= 0) continue;
        out.push({ mount: m[1], freeGB: free, totalGB: total });
    }
    return out.sort((a, b) => a.mount.localeCompare(b.mount));
}

/** The window choices, in samples at the default 1 s interval. */
export const WINDOW_CHOICES = [
    { points: 60, label: "1m" },
    { points: 120, label: "2m" },
    { points: 300, label: "5m" },
    { points: 900, label: "15m" },
] as const;
