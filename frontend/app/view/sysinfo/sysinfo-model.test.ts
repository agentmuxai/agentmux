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

    it("tolerates sub-threshold jitter without marking a break", () => {
        // A sub-tick backwards nudge is slew, not a step: keep the history and
        // do NOT insert a visible break. The one overtaken point is dropped so
        // the series still can't go non-monotonic.
        const base = 10_000_000;
        const state = [sample(base), sample(base + 1000)];
        const out = append(state, sample(base + 500));
        expect(out.map((d) => d.ts)).toEqual([base, base + 500]);
        expect(out.some((d) => d.blank === 1)).toBe(false);
    });

    // reagentx P1 (PR #2832): the first version of this fix used ONE threshold
    // for two different jobs — deciding a step happened, and deciding which
    // points may remain. A point sitting between `item.ts` and
    // `item.ts + gapThreshold` survived the filter but is still AHEAD of the
    // new sample, so the `clockStepped` branch appended
    // `[...trimmed, blank(last.ts + 1), item]` with `last.ts + 1 > item.ts` —
    // reintroducing exactly the non-monotonic series this PR exists to fix.
    // Only reachable for a PARTIAL step: bigger than the threshold, but not
    // big enough to clear every buffered point — i.e. an ordinary few-second
    // NTP correction, which is far more common than a 55-year jump.
    it("stays monotonic for a partial step that clears only some of the buffer", () => {
        const out = append([sample(8000), sample(9000), sample(10000)], sample(5000));
        expect(out.map((d) => d.ts)).toEqual([5000]);
        for (let i = 1; i < out.length; i++) {
            expect(out[i].ts).toBeGreaterThan(out[i - 1].ts);
        }
    });

    it("stays monotonic across every backwards step magnitude", () => {
        const base = 10_000_000;
        const state = [sample(base), sample(base + 1000), sample(base + 2000), sample(base + 3000)];
        // Sweep well past the gap threshold (3000ms at 1Hz) and back, so both
        // the partial-step and full-clear regimes are covered.
        for (const back of [100, 500, 1000, 2500, 3000, 3500, 4000, 5000, 10_000, 60_000]) {
            const out = append(state, sample(base + 3000 - back));
            for (let i = 1; i < out.length; i++) {
                expect(out[i].ts, `regression at back=${back}ms`).toBeGreaterThan(out[i - 1].ts);
            }
        }
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

    // codex P2 (PR #2832): a sub-threshold backwards correction in history left
    // BOTH samples in place, and bracketing the seam with `prev.ts + 1` /
    // `cur.ts - 1` blanks inherits the same inversion rather than repairing it
    // — history [10000, 8000] emitted 10000, 10001, 7999, 8000.
    it("drops the superseded segment on a sub-threshold backwards correction", () => {
        const out = reset([sample(10000), sample(8000)]);
        const ts = out.map((d) => d.ts);
        expect(ts).not.toContain(10000);
        expect(ts).toContain(8000);
        for (let i = 1; i < out.length; i++) {
            expect(out[i].ts).toBeGreaterThan(out[i - 1].ts);
        }
    });

    it("repairs arbitrary out-of-order ring content, anchored on the newest sample", () => {
        // Ring order is insertion order, so any interleaving is possible.
        const out = reset([sample(5000), sample(9000), sample(6000), sample(7000)]);
        for (let i = 1; i < out.length; i++) {
            expect(out[i].ts).toBeGreaterThan(out[i - 1].ts);
        }
        // The newest-by-arrival sample is the live one and must survive.
        expect(out[out.length - 1].ts).toBe(7000);
    });
});

describe("sampleReducer — no duplicate x", () => {
    it("omits the seam sentinel when it would collide with the new sample", () => {
        // `last.ts + 1 === item.ts`: the sentinel has nowhere to go.
        const out = append([sample(4999), sample(20000)], sample(5000));
        expect(out.map((d) => d.ts)).toEqual([4999, 5000]);
        expect(new Set(out.map((d) => d.ts)).size).toBe(out.length);
    });

    it("supersedes a buffered sample sharing the new sample's timestamp", () => {
        const out = append([sample(1000), sample(2000, 11)], sample(2000, 99));
        expect(out.map((d) => d.ts)).toEqual([1000, 2000]);
        expect(out[out.length - 1].cpu).toBe(99);
    });
});
