// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The GPU readouts' figures: when they turn the warning colour, and how bytes and percentages are written. */

/** Use or memory at or above this share turns the GPU meter's label the warning colour. */
export const GPU_WARN_PCT = 90;

/** The meter's colour: warning at 90% use or 90% memory, otherwise the bar's plain text. */
export function gpuColor(use: number | null | undefined, mem: number | null | undefined): string {
    const hot = (use ?? 0) >= GPU_WARN_PCT || (mem ?? 0) >= GPU_WARN_PCT;
    return hot ? "var(--warning-text-color)" : "var(--secondary-text-color)";
}

const GB = 1024 ** 3;
const MB = 1024 ** 2;

/** 3.2 GB, 640 MB: the panel's memory figures. */
export function formatGpuBytes(bytes: number): string {
    if (bytes >= GB) return `${(bytes / GB).toFixed(bytes >= 10 * GB ? 0 : 1)} GB`;
    if (bytes >= MB) return `${Math.round(bytes / MB)} MB`;
    return "0 MB";
}

/** A percentage as the panel and the bar print it: whole, under 0.5 as "0%". */
export function formatPct(pct: number): string {
    return `${Math.round(pct)}%`;
}

/** The width of a bar's fill, 0 to 100. */
export function barWidth(pct: number | null | undefined): number {
    return Math.min(100, Math.max(0, pct ?? 0));
}

/** An adapter's memory of one kind, as "used / total" (or just "used" where the OS gave no total), or null. */
export function memoryText(used: number | undefined, total: number | undefined): string | null {
    if (used == null) return null;
    return total != null && total > 0 ? `${formatGpuBytes(used)} / ${formatGpuBytes(total)}` : formatGpuBytes(used);
}
