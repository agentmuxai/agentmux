// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The record's side of the Memory view: what `GET /api/v1/app/record` answers (the hourly rollups and the stalls
// srv keeps on this machine, numbers and stable keys only) turned into the lines and counts the panel shows
// (docs/specs/SPEC_LONG_RUNNING_ZERO_DEGRADATION_2026_11_16.md §6, task #47820).

import type { HistoryPoint } from "./tower-model";
import { formatMem } from "./tower-util";

/** `[min, median, p95, max]` of a figure over an hour. */
export type Stat = [number, number, number, number];

export interface PageRoll {
    /** The window's label. */
    w: string;
    /** MB. */
    heap?: Stat;
    dom: Stat;
    rows: Stat;
}

/** One finished hour, as `crates/srv-core/src/backend/record.rs` writes it. */
export interface RollupLine {
    /** The hour's start, ms. */
    h: number;
    n: number;
    v: string;
    /** The uptime of the hour's last line, seconds. */
    up_s: number;
    /** Times srv started again inside the hour. */
    restarts: number;
    /** When the hour's last line was taken; absent in an older rollup. */
    last_t?: number;
    am?: Stat;
    parts: [string, Stat][];
    pages?: PageRoll[];
}

export interface StallLine {
    t: number;
    kind: string;
    ms?: number;
}

export interface RecordAnswer<T> {
    /** Lines the window holds in all: more than `lines` when one answer's limit cut it. */
    matched: number;
    lines: T[];
}

export type LongSpan = "1d" | "7d" | "30d";
export const SPAN_MS: Record<LongSpan, number> = { "1d": 86_400_000, "7d": 7 * 86_400_000, "30d": 30 * 86_400_000 };
const HOUR_MS = 3_600_000;

/** What the chart can follow: AgentMux's total, each part, and the page of the window that holds the most. */
export type LongKey = "am" | "core" | "panes" | "agents" | "heap" | "dom" | "rows";
export const LONG_KEYS: LongKey[] = ["am", "core", "panes", "agents", "heap", "dom", "rows"];

/** Whether the figure is one window's page (its JS heap, elements or rows), not a process total. */
export const isPageKey = (k: LongKey) => k === "heap" || k === "dom" || k === "rows";

/** The window whose page the chart follows: the one the newest hour that had pages ranks first (the record keeps
 *  the three with the largest heap). */
export function pageWindow(lines: readonly RollupLine[]): string | undefined {
    for (let i = lines.length - 1; i >= 0; i--) {
        const first = lines[i].pages?.[0];
        if (first) return first.w;
    }
    return undefined;
}

/** The median of `key` in each hour, oldest first: the typical level of the hour, which a spike does not move.
 *  An hour that did not name the figure has no point. */
export function pointsOf(lines: readonly RollupLine[], key: LongKey): HistoryPoint[] {
    const w = isPageKey(key) ? pageWindow(lines) : undefined;
    const out: HistoryPoint[] = [];
    for (const l of lines) {
        let stat: Stat | undefined;
        if (key === "am") stat = l.am;
        else if (isPageKey(key)) {
            const page = l.pages?.find((p) => p.w === w);
            stat = key === "heap" ? page?.heap : page?.[key];
        } else stat = l.parts.find(([name]) => name === key)?.[1];
        if (stat) out.push({ t: l.h, mem: stat[1] });
    }
    return out;
}

/** How a figure of `key` reads: bytes for memory, MB for a heap, a plain count for elements and rows. */
export function formatLong(key: LongKey, v: number | undefined): string {
    if (v == null) return "—";
    if (key === "heap") return `${Math.round(v)} MB`;
    if (isPageKey(key)) return Math.round(v).toLocaleString();
    return formatMem(v);
}

/** The least the chart's scale reaches, so a quiet figure stays flat. */
export function floorOf(key: LongKey): number {
    return key === "heap" ? 64 : key === "dom" ? 2_000 : key === "rows" ? 100 : 64 * 1024 * 1024;
}

/** The last time of an hour's last line, and whether it is the line's own (an older rollup has only the hour). */
function lastTime(l: RollupLine): [number, boolean] {
    return l.last_t ? [l.last_t, true] : [l.h, false];
}

/** How many times srv started again over `lines`: those each hour counted inside itself, and those between two
 *  hours, where the later hour's uptime is less than the earlier one's plus the time between the two last lines.
 *  Two minutes of room where both lines' times are known, 15 minutes otherwise. An hour that counted a restart
 *  itself is why its uptime fell short, so it is not counted twice. */
export function restartsIn(lines: readonly RollupLine[]): number {
    const inside = lines.reduce((sum, l) => sum + l.restarts, 0);
    let between = 0;
    for (let i = 1; i < lines.length; i++) {
        const [a, b] = [lines[i - 1], lines[i]];
        if (b.restarts > 0) continue;
        const [[tb, exactB], [ta, exactA]] = [lastTime(b), lastTime(a)];
        const gapS = Math.max(0, tb - ta) / 1000;
        const room = exactA && exactB ? 120 : 900;
        if (b.up_s + room < a.up_s + gapS) between++;
    }
    return inside + between;
}

export interface StallCount {
    count: number;
    longestMs: number;
}

/** The stalls by kind; `cut` when the answer held fewer than the window has (counts are of the newest). */
export function stallCounts(events: RecordAnswer<StallLine>): { ui: StallCount; srv: StallCount; cut: boolean } {
    const of = (kind: string): StallCount => {
        const mine = events.lines.filter((e) => e.kind === kind);
        return { count: mine.length, longestMs: Math.max(0, ...mine.map((e) => e.ms ?? 0)) };
    };
    return { ui: of("ui_stall"), srv: of("srv_stall"), cut: events.matched > events.lines.length };
}

/** "55 s", "4 min", "1 h 5 min". */
export function formatSpan(ms: number): string {
    const s = Math.round(ms / 1000);
    if (s < 60) return `${s} s`;
    const min = Math.round(s / 60);
    return min < 60 ? `${min} min` : `${Math.floor(min / 60)} h${min % 60 ? ` ${min % 60} min` : ""}`;
}

/** The newest hour's page of each window it names, for the counters beside the chart. */
export function latestPages(lines: readonly RollupLine[]): PageRoll[] {
    for (let i = lines.length - 1; i >= 0; i--) {
        const pages = lines[i].pages;
        if (pages?.length) return pages;
    }
    return [];
}

/** The query for `GET /api/v1/app/record` over the last `span`. */
export function recordQuery(kind: "rollup" | "events", span: LongSpan, now: number): string {
    const from = Math.max(0, now - SPAN_MS[span]);
    const limit = kind === "rollup" ? Math.ceil(SPAN_MS[span] / HOUR_MS) + 2 : 500;
    return `kind=${kind}&from_ms=${from}&limit=${limit}`;
}
