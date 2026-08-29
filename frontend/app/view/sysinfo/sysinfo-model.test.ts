// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { sampleReducer } from "./sysinfo-model";
import type { DataItem } from "./sysinfo-types";

const INTERVAL = 1;
const INTERVAL_MS = 1000;
const NUM_POINTS = 120;

function sample(ts: number, cpu = 10): DataItem {
    return { ts, cpu };
}

function append(state: DataItem[], item: DataItem): DataItem[] {
    return sampleReducer(state, { type: "APPEND", item, intervalSecs: INTERVAL, numPoints: NUM_POINTS });
}

function reset(items: DataItem[]): DataItem[] {
    return sampleReducer([], { type: "RESET", items, intervalSecs: INTERVAL, numPoints: NUM_POINTS });
}

/** The exact live incident: srv ran under a 2081 clock, then the clock was
 *  corrected back to 2026 with no restart. */
const PRE_STEP = new Date("2081-02-05T09:58:22.373Z").getTime();
const POST_STEP = new Date("2026-08-29T06:28:33.964Z").getTime();

describe("sampleReducer — APPEND, ordinary forward time", () => {
    it("appends a contiguous sample", () => {
        const state = [sample(1000), sample(2000)];
        expect(append(state, sample(3000)).map((d) => d.ts)).toEqual([1000, 2000, 3000]);
    });

    it("trims samples older than the visible window", () => {
        const old = sample(1000);
        const recent = sample(1000 + INTERVAL_MS * (NUM_POINTS + 1));
        const next = sample(recent.ts + INTERVAL_MS);
        const out = append([old, recent], next);
        expect(out.map((d) => d.ts)).toEqual([recent.ts, next.ts]);
    });

    it("zero-order holds across 1-3 missed ticks", () => {
        const state = [sample(1000, 42)];
        const out = append(state, sample(4000, 7));
        expect(out.map((d) => d.ts)).toEqual([1000, 2000, 3000, 4000]);
        expect(out[1].cpu).toBe(42);
        expect(out[2].cpu).toBe(42);
        expect(out[3].cpu).toBe(7);
    });

    it("inserts a NaN sentinel across a long forward break", () => {
        const state = [sample(1000)];
        const out = append(state, sample(20000));
        expect(out).toHaveLength(3);
        expect(out[1].blank).toBe(1);
        expect(Number.isNaN(out[1].cpu as number)).toBe(true);
    });
});

// Regression: wall-clock `ts` can step BACKWARDS (NTP correction, manual set,
// VM resume). Before the fix the reducer had two forward-only assumptions:
//
//  1. `cutoffTs = item.ts - intervalMs * (numPoints + 1)` with
//     `state.filter(d => d.ts >= cutoffTs)` — a pre-step point's ts is LARGER
//     than any post-step cutoff, so it passed the filter forever and never
//     aged out.
//  2. Both gap branches tested `gap > ...`, and a backwards jump makes `gap`
//     negative, so neither fired and no break was marked — the series went
//     silently non-monotonic in x.
describe("sampleReducer — APPEND across a backwards clock step", () => {
    it("drops points stamped after the new sample instead of retaining them forever", () => {
        const state = [sample(PRE_STEP - 2000), sample(PRE_STEP - 1000), sample(PRE_STEP)];
        const out = append(state, sample(POST_STEP));
        expect(out.map((d) => d.ts)).toEqual([POST_STEP]);
    });

    it("keeps the buffer bounded and monotonic as samples continue after the step", () => {
        let state: DataItem[] = [sample(PRE_STEP - 1000), sample(PRE_STEP)];
        for (let i = 0; i < 10; i++) {
            state = append(state, sample(POST_STEP + i * INTERVAL_MS));
        }
        expect(state).toHaveLength(10);
        for (let i = 1; i < state.length; i++) {
            expect(state[i].ts).toBeGreaterThan(state[i - 1].ts);
        }
    });

    it("marks a break rather than connecting the line across the seam", () => {
        // A partial step — small enough that some points survive the trim, so
        // there is a real seam to mark (the full-step case above has nothing
        // left to connect from).
        const base = 10_000_000;
        const state = [sample(base), sample(base + 8000)];
        const out = append(state, sample(base + 1000));
        const blanks = out.filter((d) => d.blank === 1);
        expect(blanks).toHaveLength(1);
        expect(out[out.length - 1].ts).toBe(base + 1000);
        // The far-ahead point is gone; the retained one is genuinely older.
        expect(out.some((d) => d.ts === base + 8000)).toBe(false);
    });

    it("tolerates sub-threshold jitter without discarding the buffer", () => {
        // A ~1s backwards nudge is slew, not a step: keep the history.
        const base = 10_000_000;
        const state = [sample(base), sample(base + 1000)];
        const out = append(state, sample(base + 500));
        expect(out.map((d) => d.ts)).toEqual([base, base + 1000, base + 500]);
        expect(out.some((d) => d.blank === 1)).toBe(false);
    });
});

describe("sampleReducer — RESET", () => {
    // RESET pads the start of the window with leading blanks when the oldest
    // real sample is newer than the cutoff (pre-existing behaviour, so the
    // x-domain is filled) — assert on the real samples, not the padding.
    const realSamples = (out: DataItem[]) => out.filter((d) => d.blank !== 1).map((d) => d.ts);

    it("keeps a contiguous history intact", () => {
        const items = [sample(1000), sample(2000), sample(3000)];
        expect(realSamples(reset(items))).toEqual([1000, 2000, 3000]);
    });

    it("inserts sentinels around a forward gap in history", () => {
        const items = [sample(1000), sample(60000)];
        const out = reset(items);
        const blankTs = out.filter((d) => d.blank === 1).map((d) => d.ts);
        // The pair bracketing the gap itself, distinct from the leading pad.
        expect(blankTs).toContain(1001);
        expect(blankTs).toContain(59999);
        expect(realSamples(out)).toEqual([1000, 60000]);
    });

    it("drops pre-step points from a history that spans a backwards clock step", () => {
        // Ring order is insertion order: pre-step samples first, then post-step.
        const items = [
            sample(PRE_STEP - 1000),
            sample(PRE_STEP),
            sample(POST_STEP),
            sample(POST_STEP + INTERVAL_MS),
        ];
        const out = reset(items);
        expect(out.every((d) => d.ts <= POST_STEP + INTERVAL_MS)).toBe(true);
        expect(out.some((d) => d.ts === PRE_STEP)).toBe(false);
    });

    it("never returns a non-monotonic series", () => {
        const items = [sample(PRE_STEP), sample(POST_STEP), sample(POST_STEP + INTERVAL_MS)];
        const out = reset(items);
        for (let i = 1; i < out.length; i++) {
            expect(out[i].ts).toBeGreaterThan(out[i - 1].ts);
        }
    });
});
