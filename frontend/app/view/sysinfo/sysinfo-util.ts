// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { DataItem } from "./sysinfo-types";

export function convertWaveEventToDataItem(event: WaveEvent): DataItem {
    const eventData: TimeSeriesData = event.data;
    if (eventData == null || eventData.ts == null || eventData.values == null) {
        return null;
    }
    const dataItem: DataItem = { ts: eventData.ts };
    for (const key in eventData.values) {
        dataItem[key] = eventData.values[key];
    }
    return dataItem;
}

export function resolveDomainBound(value: number | string, dataItem: DataItem): number | undefined {
    if (typeof value == "number") {
        return value;
    } else if (typeof value == "string") {
        return dataItem?.[value];
    } else {
        return undefined;
    }
}

/**
 * Get the gap detection threshold in ms. Uses 2x the configured interval
 * (minimum 3000ms) so that normal jitter at max interval (2.0s) doesn't
 * trigger spurious reloads.
 */
export function getGapThresholdMs(configIntervalSecs: number): number {
    const intervalMs = (configIntervalSecs || 1.0) * 1000;
    return Math.max(3000, intervalMs * 2.5);
}

/**
 * Compute a dynamic (auto-scaled) y-max for a metric with no natural
 * ceiling (network/disk throughput) or one where the fixed ceiling wastes
 * most of the chart when actual usage sits well below it (memory).
 *
 * Domain source is the CURRENTLY VISIBLE window only (`plotData`, already
 * trimmed to the chart's target length by the reducer) — not the full
 * history — so the axis reflects what's on screen. Deliberately does NOT
 * track any separate "hold" state across renders: since old samples fall
 * out of `plotData` naturally as time advances, a spike keeps influencing
 * the ceiling for as long as it's still in the visible window, then the
 * axis eases back down as it scrolls out — a simple, real recompute gets
 * "doesn't snap back down instantly" behavior for free, no extra decay
 * bookkeeping needed.
 *
 * `hardCap` (from `maxy`, if the metric has one, e.g. memory's
 * `mem:total`) is enforced last — the auto-scaled value never exceeds it,
 * since some ceilings (like total RAM) are real physical limits, not just
 * a display convenience.
 *
 * See docs/reports/REPORT_SYSINFO_COMBINED_CHART_RESEARCH_2026_08_17.md.
 */
export function computeAutoMaxY(
    plotData: DataItem[],
    yval: string,
    floor: number,
    hardCap: number | undefined,
    paddingFraction = 0.15
): number {
    let observedMax = 0;
    for (const item of plotData) {
        const v = item?.[yval];
        if (typeof v === "number" && Number.isFinite(v) && v > observedMax) {
            observedMax = v;
        }
    }
    let padded = observedMax * (1 + paddingFraction);
    padded = Math.max(padded, floor);
    if (hardCap != null && Number.isFinite(hardCap)) {
        padded = Math.min(padded, hardCap);
    }
    return padded;
}
