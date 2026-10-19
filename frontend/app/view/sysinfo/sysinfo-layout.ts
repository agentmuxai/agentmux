// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Where a Sysinfo chart's text goes, as pure rules the simple grid and the
 * Advanced focus view share: which chart draws the time labels, which draws y
 * labels, how tall a chart must be to show axes at all, and one y-axis width per
 * column so plots line up. All sizes are at 100% and scale with the pane's zoom.
 * docs/specs/SPEC_SYSINFO_TEXT_ZOOM_DENSITY_AND_ONE_ZOOM_PATH_2026_10_18.md §5.
 */

import { createEffect, createSignal, on, type Accessor } from "solid-js";

/** The axis label font at 100% zoom, in px. */
export const AXIS_FONT_PX = 12;
/** Least plot height (after its text) that still gets axes; below it a chart is a bare sparkline. */
export const MIN_PLOT_PX = 28;
/** Tick marks and the gap between a tick and its label (graphics: they don't zoom). */
export const TICK_PX = 3;
export const TICK_GAP_PX = 3;
/** A time axis without labels: its tick marks only. */
export const BARE_AXIS_PX = 4;

/** The axis label font size at a zoom. */
export const axisFontPx = (zoom: number): number => Math.max(6, Math.round(AXIS_FONT_PX * zoom));

/** Room above the plot for the top y label: half a label, plus the 1px text inset. */
export const topPaddingPx = (zoom: number): number => Math.ceil(axisFontPx(zoom) / 2) + 1;

/** Height of a time axis: its labels and ticks, or ticks only. */
export const timeAxisPx = (zoom: number, labels: boolean): number =>
    labels ? Math.ceil(axisFontPx(zoom) * 1.25) + TICK_PX + TICK_GAP_PX : BARE_AXIS_PX;

/** Least height for a chart that shows its axes; a shorter one is drawn bare (§5 C.2 rule 8). */
export const minFullChartPx = (zoom: number, timeLabels: boolean): number =>
    Math.round(MIN_PLOT_PX * zoom) + topPaddingPx(zoom) + timeAxisPx(zoom, timeLabels);

/** Only the bottom chart of each column draws time labels (§5 C.2 rule 2). Charts fill rows left to right. */
export function showsTimeAxis(index: number, count: number, columns: number): boolean {
    const rows = Math.ceil(count / columns);
    return Math.floor(index / columns) === rows - 1;
}

/** When every chart in a row has the same y range, only the leftmost draws y labels (§5 C.2 rule 3). */
export function showsYLabels(index: number, columns: number, sameRange: boolean): boolean {
    return !sameRange || index % columns === 0;
}

/** Whether a chart sits in the top row, under the floating buttons (§5 C.2 rule 4). */
export const inTopRow = (index: number, columns: number): boolean => index < columns;

/** Measures every simple-view metric on the same 0–100% scale (the cores, or CPU alone)? */
export const sharesPercentRange = (keys: readonly string[]): boolean =>
    keys.length > 1 && keys.every((k) => /^cpu(:\d+)?$/.test(k));

/**
 * One y-axis width per column, so the plots in a column start at the same x and
 * time reads straight down (§5 C.2 rule 6). Each chart reports the width its own
 * labels need; each gets back its column's widest. Cleared when `reset` changes
 * (a new set of charts), so a chart that left doesn't keep a column wide.
 */
export function createAxisColumns(reset: Accessor<unknown>) {
    const [widths, setWidths] = createSignal<Record<string, number>>({});
    createEffect(on(reset, () => setWidths({}), { defer: true }));
    return {
        report(column: number, key: string, width: number): void {
            const id = `${column}:${key}`;
            setWidths((prev) => (prev[id] === width ? prev : { ...prev, [id]: width }));
        },
        widthFor(column: number): number {
            let max = 0;
            for (const [id, w] of Object.entries(widths())) if (id.startsWith(`${column}:`)) max = Math.max(max, w);
            return max;
        },
    };
}
