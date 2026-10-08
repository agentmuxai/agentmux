// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The words for the performance health signals shown in the backend status
 * panel: a short label per kind and the full advice. Moved from the old
 * top-of-window banner; the advice is unchanged.
 * SPEC_RAM_PAGEFILE_PRESSURE_SPLIT_2026_08_07, ANALYSIS_SRV_HTTP_STALL_IO_DRIVER_STARVATION_2026_09_26 §8.2,
 * REPORT_PERFORMANCE_INDICATORS_TO_STATUS_BAR_2026_10_08.
 */

import type { ActiveHealthLevel, HealthKind, HealthPayload } from "./health-signals";

const LABEL: Record<HealthKind, string> = {
    ram: "Low RAM",
    pagefile: "Low page file",
    backend: "Slow backend",
};

const RAM_MESSAGE: Record<ActiveHealthLevel, string> = {
    warn: "System RAM is running low. Performance may degrade; closing some windows or apps will help.",
    critical: "System RAM is critically low. Closing some windows or other apps will keep AgentMux responsive.",
};

const PAGEFILE_MESSAGE: Record<ActiveHealthLevel, string> = {
    warn: "Virtual memory (page file) is running low.",
    critical: "Virtual memory (page file) is critically low — an out-of-memory crash is imminent.",
};

/** Disk-free % below which a system-managed page file is treated as
 *  practically stuck even though Windows would like to grow it — matches the
 *  ~15-20% free-disk framing in SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29 §5.2. */
const PAGEFILE_DISK_LOW_PCT = 20;

/** The heading of a signal's notice; the backend's carries its average. */
export function healthLabel(kind: HealthKind, payload?: HealthPayload): string {
    const avg = kind === "backend" ? formatAvg(payload?.avg_ms) : "";
    return avg ? `${LABEL[kind]} ${avg}` : LABEL[kind];
}

/** The disk/OS-managed-aware guidance appended to the Page File message —
 *  the "may the OS handle it, or not" distinction from
 *  SPEC_RAM_PAGEFILE_PRESSURE_SPLIT_2026_08_07 §4. Empty string if the
 *  host didn't report disk context (e.g. a read failure) — no guidance is
 *  better than a guess. */
export function pagefileGuidance(systemManaged?: boolean, diskFreePct?: number): string {
    if (systemManaged === false) {
        return " Your page file has a fixed size and won't grow automatically — free up disk space or increase its size in Windows settings.";
    }
    if (systemManaged === true && diskFreePct !== undefined && diskFreePct < PAGEFILE_DISK_LOW_PCT) {
        return " Windows can't grow your page file because disk space is low — free up disk space now to avoid a crash.";
    }
    if (systemManaged === true) {
        return " Windows can expand virtual memory automatically, but performance may dip in the meantime.";
    }
    return "";
}

/** "1.8s" from a rolling-average ms value; empty when the host sent none. */
export function formatAvg(avgMs?: number): string {
    if (avgMs === undefined || !Number.isFinite(avgMs)) return "";
    return `${(avgMs / 1000).toFixed(1)}s`;
}

export function backendMessage(level: ActiveHealthLevel, avgMs?: number): string {
    const avg = formatAvg(avgMs);
    const avgPart = avg ? ` (avg ${avg})` : "";
    return level === "critical"
        ? `AgentMux is barely responding${avgPart} — it may restart itself to recover.`
        : `AgentMux is responding slowly${avgPart}. Heavy CPU use by other processes, including agents' builds, can cause this.`;
}

/** The full advice for a kind and level. RAM never gets disk guidance (the
 *  concept doesn't apply to physical RAM). */
export function messageFor(kind: HealthKind, level: ActiveHealthLevel, payload: HealthPayload): string {
    if (kind === "ram") return RAM_MESSAGE[level];
    if (kind === "backend") return backendMessage(level, payload.avg_ms);
    return PAGEFILE_MESSAGE[level] + pagefileGuidance(payload.system_managed, payload.disk_free_pct);
}

/** "for 4 min" style duration of an episode; "just now" under a minute. */
export function formatSince(elapsedMs: number): string {
    const mins = Math.floor(Math.max(0, elapsedMs) / 60_000);
    if (mins < 1) return "just now";
    if (mins < 60) return `for ${mins} min`;
    const hours = Math.floor(mins / 60);
    const rest = mins % 60;
    return rest ? `for ${hours} h ${rest} min` : `for ${hours} h`;
}
