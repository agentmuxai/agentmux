// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Low free space on the volume that holds AgentMux's data, for a page whose host does not push the signal itself
 * (AgentMux Web: `hostHas("hostHealthSignals")` is false). The desktop's heartbeat measures the drive and pushes a
 * `health-signal` event (crates/cef/src/memory_heartbeat/low_disk.rs); a browser tab has no such host, so the page
 * reads the same drive from the system info the Disk pill already shows (`disk:watch:*`, sysinfo.rs).
 *
 * The lines are not copied: both sides read `low-disk-levels.json`, so the desktop and the web warn at the same free
 * space. The step logic below mirrors `classify` in low_disk.rs (entering a level takes being under either of its
 * lines; leaving it takes being clear of both by a margin), and the tests pin the two to the same cases.
 */

import levels from "@/app/store/low-disk-levels.json";

export type DiskLevel = "normal" | "warn" | "critical";

/** One reading of the watched volume. */
export interface DiskReading {
    freeGb: number;
    totalGb: number;
    freePct: number;
}

export interface DiskLevels {
    warnPct: number;
    warnGb: number;
    criticalPct: number;
    criticalGb: number;
    clearPct: number;
    clearGb: number;
}

export const DISK_LEVELS: Readonly<DiskLevels> = levels;

const below = (r: DiskReading, pct: number, gb: number): boolean => r.freePct < pct || r.freeGb < gb;

const severity = (level: DiskLevel): number => (level === "critical" ? 2 : level === "warn" ? 1 : 0);

/** The level for `r`, given the level before: leaving a level needs `clearPct` and `clearGb` of headroom over its line. */
export function classifyDisk(r: DiskReading, prev: DiskLevel, l: Readonly<DiskLevels> = DISK_LEVELS): DiskLevel {
    const critical =
        prev === "critical" ? below(r, l.criticalPct + l.clearPct, l.criticalGb + l.clearGb) : below(r, l.criticalPct, l.criticalGb);
    if (critical) return "critical";
    const warn = prev === "normal" ? below(r, l.warnPct, l.warnGb) : below(r, l.warnPct + l.clearPct, l.warnGb + l.clearGb);
    return warn ? "warn" : "normal";
}

/** What one reading did. `crossed` is the once-per-crossing event: the level got worse than it was. */
export interface DiskStep {
    level: DiskLevel;
    crossed: boolean;
}

/** Folds readings into levels. A steady level, and a recovery, are not crossings; the next drop under a line is. */
export function createLowDiskTracker(l: Readonly<DiskLevels> = DISK_LEVELS) {
    let level: DiskLevel = "normal";
    return {
        level: (): DiskLevel => level,
        observe(r: DiskReading): DiskStep {
            const next = classifyDisk(r, level, l);
            const crossed = severity(next) > severity(level);
            level = next;
            return { level: next, crossed };
        },
    };
}

/** A reading from the sysinfo values, or null when the watched volume is not in them (a torn first tick). */
export function readingFromSysinfo(vals: Record<string, number>): DiskReading | null {
    const freeGb = vals["disk:watch:free_gb"];
    const totalGb = vals["disk:watch:total_gb"];
    const freePct = vals["disk:watch:free_pct"];
    if (freeGb == null || totalGb == null || freePct == null || !(totalGb > 0)) return null;
    return { freeGb, totalGb, freePct };
}

/** "222 GB" or "9.5 GB": one decimal under 100 (the desktop's notice words it the same way). */
export function formatFreeGb(gb: number): string {
    return gb < 100 ? `${gb.toFixed(1)} GB` : `${Math.round(gb)} GB`;
}

export interface LowDiskText {
    title: string;
    free: string;
    why: string;
    how: string;
    space: string;
}

/**
 * The words of the notice: how much is free and where, why it matters, how to make room, and where the space may be
 * going. The volume is named by the label the Disk pill uses; the page cannot see inside it, so "what takes the space"
 * points at the one measurement AgentMux has, the history sizes.
 */
export function lowDiskText(level: Exclude<DiskLevel, "normal">, r: DiskReading, label?: string): LowDiskText {
    const where = label ? ` on ${label}` : "";
    return {
        title: level === "critical" ? "Disk space is almost gone" : "Disk space is running low",
        free: `${formatFreeGb(r.freeGb)} free${where}, ${r.freePct.toFixed(1)}% of ${formatFreeGb(r.totalGb)}.`,
        why:
            level === "critical"
                ? "When the volume is full, saving files fails and so do AgentMux's own saves, agents' included."
                : "On a nearly full volume, saves slow down and can start to fail.",
        how: "Delete files you no longer need. If this workspace runs in AgentMux Cloud, you can make the data volume bigger on its card in the Cloud Console.",
        space: "Agents' history is kept on this volume; the History pane shows how much space it takes.",
    };
}
