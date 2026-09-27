// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { DataItem } from "./sysinfo-types";

export function convertMuxEventToDataItem(event: MuxEvent): DataItem {
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

/** Layout margins for one panel's `Plot.plot()` call, in pixels. */
export type PlotMargins = {
    marginTop: number;
    marginRight: number;
    marginBottom: number;
    marginLeft: number;
};

/**
 * Margins tight enough for this panel's actual content, instead of trusting
 * Observable Plot's auto-computed margins — those are sized for a full-page
 * chart with room to spare, which is disproportionately wasteful on these
 * small (as little as ~100px tall) grid panels.
 *
 * A sparkline hides its axis entirely (`axis: !sparkline` in SingleLinePlot),
 * so it needs only a sliver of room to keep the hover dot/tip from clipping
 * at the edge — no tick text, no axis label.
 *
 * A titled panel (`title` — the metric name, top-left) needs enough top
 * margin to clear that text; an untitled one needs only the same sliver as a
 * sparkline's top edge.
 *
 * With both axis labels moved to sit at the origin (see
 * buildPlotAxisLabelOptions below), neither one claims its own reserved
 * band past the last tick, so
 * marginRight/marginBottom only need to fit the tick text itself plus a few
 * px of clearance for the pointer dot at the data's edge.
 */
export function computePlotMargins(sparkline: boolean, title: boolean): PlotMargins {
    if (sparkline) {
        return { marginTop: 4, marginRight: 4, marginBottom: 4, marginLeft: 4 };
    }
    return {
        marginTop: title ? 16 : 6,
        marginRight: 10,
        // Tick text row + axis line; the x-axis label shares that row (labelAnchor: "left") rather than adding one below it.
        marginBottom: 20,
        // Up to 3-digit tick values + a leading "-"; the y-axis label shares that column (labelAnchor: "bottom") rather than adding a band above it.
        marginLeft: 30,
    };
}

/**
 * x/y scale options that put both axis labels at the origin (the corner
 * where the lowest x value meets the lowest y value), in the same
 * alignment the tick VALUES already use there, instead of Plot's default:
 * the x label past the last tick (`labelAnchor: "right"`, the default for a
 * quantitative/temporal x scale) and the y label above the first tick
 * (`labelAnchor: "top"`, the default for a quantitative y scale).
 *
 * `labelAnchor: "left"` on x anchors the label at the axis's start (time's
 * minimum, which sits at the origin) instead of its end; `labelAnchor:
 * "bottom"` on y anchors it at the axis's start (the domain minimum, at the
 * origin) instead of its top. Both then read from the same corner as the
 * first tick's own value, rather than the label living somewhere else on
 * the axis.
 */
export function buildPlotAxisLabelOptions(): { x: { labelAnchor: "left" }; y: { labelAnchor: "bottom" } } {
    return { x: { labelAnchor: "left" }, y: { labelAnchor: "bottom" } };
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
