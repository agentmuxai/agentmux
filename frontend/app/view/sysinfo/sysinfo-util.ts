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
