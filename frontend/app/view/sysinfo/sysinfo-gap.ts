// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the chart draws between two samples that are further apart than a few
 * intervals. srv stamps each sample with the time its rates average over
 * (`sample:window_ms`). A sample that came late (srv's sampler was held up, not
 * lost in delivery) has a window as long as the gap before it: its CPU, network
 * and disk figures are the average over that whole time, so they are drawn as a
 * step across it. A gap its sample doesn't cover (samples missing, srv down, the
 * machine asleep) is a break, as before, and srv's history may fill it.
 * #4812 draws missing samples as breaks; #4829 backfills a stall's gap.
 */

import type { DataItem } from "./sysinfo-types";

/** The sample key for the milliseconds its rates average over (srv's `SAMPLE_WINDOW_KEY`). */
export const SAMPLE_WINDOW_KEY = "sample:window_ms";

/** A window may fall this far short of its gap (clock rounding between srv and the pane) and still cover it. */
const COVER_SLACK_MS = 500;
/** A gap longer than this is an outage, not a late tick, whatever its window says (a sleeping machine, a hung srv). */
export const MAX_COVERED_GAP_MS = 15000;

/** Figures that average over the sample's window; the others (memory) are readings of one moment. */
const isWindowAverage = (key: string): boolean =>
    key === "cpu" || /^cpu:\d+$/.test(key) || key.startsWith("net:bytes") || /^disk:(read|write|total)$/.test(key);

/** Whether `cur`'s own window spans the whole gap since `prev`: srv measured the time between them. */
export function coversGap(prev: DataItem, cur: DataItem): boolean {
    const gap = cur.ts - prev.ts;
    const window = cur[SAMPLE_WINDOW_KEY];
    return typeof window === "number" && gap <= MAX_COVERED_GAP_MS && window >= gap - COVER_SLACK_MS;
}

/** `blank(ts)` makes a gap marker; `thresholdMs` is the gap past which the chart needs to join or break. */
export type GapMarker = (ts: number) => DataItem;

/**
 * What goes between `prev` and `cur`, which are more than the threshold apart:
 * a step point just after `prev` carrying `cur`'s averages, if `cur` covers the gap; else a break at both ends.
 */
export function gapBetween(prev: DataItem, cur: DataItem, blank: GapMarker): DataItem[] {
    if (!coversGap(prev, cur)) return [blank(prev.ts + 1), blank(cur.ts - 1)];
    const step: DataItem = { ts: prev.ts + 1 };
    for (const key of Object.keys(cur)) {
        if (key === "ts") continue;
        step[key] = isWindowAverage(key) ? cur[key] : (prev[key] ?? cur[key]);
    }
    return [step];
}
