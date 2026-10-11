// Copyright 2025, Command Line Inc.
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
 * With no axis labels (see buildPlotAxisLabelOptions below), neither axis
 * claims a reserved band past the last tick, so marginRight/marginBottom only
 * need to fit the tick text itself. The right margin is 0: the chart ends flush
 * against the pane's edge.
 */
export function computePlotMargins(sparkline: boolean, title: boolean): PlotMargins {
    if (sparkline) {
        return { marginTop: 4, marginRight: 4, marginBottom: 4, marginLeft: 4 };
    }
    return {
        marginTop: title ? 16 : 6,
        // Flush: the data runs to the pane's right edge.
        marginRight: 0,
        // Tick text row + axis line.
        marginBottom: 20,
        // Up to 3-digit tick values + a leading "-".
        marginLeft: 30,
    };
}

/** Gap between the plot's left edge and the widest y tick label (the pane's own 1px border adds to it). */
export const PLOT_EDGE_PAD_PX = 1;
/** What Plot leaves between a y tick label and the axis: tick length (6) + padding (3). */
export const PLOT_TICK_GUTTER_PX = 9;

/**
 * The left margin that puts the widest y tick label `PLOT_EDGE_PAD_PX` from the
 * pane's left edge: that padding, the label, and the tick gutter. Fed with the
 * measured width of the rendered labels, so it fits "100" (CPU) and wider ones
 * (rates with units) alike. Falls back to `computePlotMargins`' default when
 * nothing could be measured (no layout, e.g. jsdom, or a hidden pane).
 */
export function computeLeftMarginForTicks(maxLabelWidthPx: number): number {
    if (!Number.isFinite(maxLabelWidthPx) || maxLabelWidthPx <= 0) return computePlotMargins(false, false).marginLeft;
    return PLOT_EDGE_PAD_PX + Math.ceil(maxLabelWidthPx) + PLOT_TICK_GUTTER_PX;
}

/**
 * x/y scale options for the axis labels: none. The ticks already say it's
 * time (clock times along x) and the panel title names the metric, so a
 * "time" / "%" label is redundant chrome in a panel this small. (#3937 had
 * moved them to the origin; the repo owner then asked for them gone.)
 * `label: null` is how Plot suppresses a label — `undefined` would fall back
 * to the channel name.
 */
export function buildPlotAxisLabelOptions(): { x: { label: null }; y: { label: null } } {
    return { x: { label: null }, y: { label: null } };
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

/** Half the width of the widest x tick label ("12:59 AM"), plus a px of clearance. */
export const X_TICK_HALF_LABEL_PX = 26;

/**
 * Whether the tick at time `d` has room for its (centred) label before the
 * plot's right edge. With the chart flush against the pane (marginRight 0), the
 * last tick can sit closer to the edge than half its label is wide, which cuts
 * the label off ("9:41 A"); such a tick's label is left blank instead.
 */
export function xTickFitsBeforeRightEdge(
    d: number,
    minX: number,
    maxX: number,
    width: number,
    marginLeft: number,
    marginRight: number
): boolean {
    if (!(maxX > minX)) return true;
    const x = marginLeft + ((d - minX) / (maxX - minX)) * (width - marginLeft - marginRight);
    return width - x >= X_TICK_HALF_LABEL_PX;
}
