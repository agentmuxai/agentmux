// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { sampleReducer } from "./sysinfo-model";
import { coversGap, MAX_COVERED_GAP_MS, SAMPLE_WINDOW_KEY } from "./sysinfo-gap";
import type { DataItem } from "./sysinfo-types";

const sample = (ts: number, cpu: number, window = 1000, mem = 8): DataItem => ({
    ts,
    cpu,
    "mem:used": mem,
    [SAMPLE_WINDOW_KEY]: window,
});
const blanks = (rows: DataItem[]) => rows.filter((r) => r.blank === 1);
const append = (state: DataItem[], item: DataItem) =>
    sampleReducer(state, { type: "APPEND", item, intervalSecs: 1, numPoints: 120 });

describe("a late tick", () => {
    it("covers its gap when its window is as long as the gap", () => {
        expect(coversGap(sample(1000, 10), sample(6000, 90, 5000))).toBe(true);
        expect(coversGap(sample(1000, 10), sample(6000, 90, 4600))).toBe(true);
    });

    it("is drawn as a step, with no break: the late sample's average from the end of the last one", () => {
        let rows = append([], sample(1000, 10));
        rows = append(rows, sample(2000, 12));
        rows = append(rows, sample(7000, 96, 5000, 9));
        expect(blanks(rows)).toHaveLength(0);
        const step = rows[rows.length - 2];
        expect(step.ts).toBe(2001);
        expect(step.cpu).toBe(96);
        expect(rows[rows.length - 1].cpu).toBe(96);
        // Memory is a reading of one moment: the line runs straight, not stepped.
        expect(step["mem:used"]).toBe(8);
    });

    it("is also drawn as a step when history is laid out", () => {
        const rows = sampleReducer([], {
            type: "RESET",
            items: [sample(1000, 10), sample(2000, 12), sample(7000, 96, 5000)],
            intervalSecs: 1,
            numPoints: 120,
        });
        expect(blanks(rows.filter((r) => r.ts >= 1000))).toHaveLength(0);
    });
});

describe("a missed tick", () => {
    it("breaks when the sample's window is only one interval: samples are missing", () => {
        expect(coversGap(sample(1000, 10), sample(7000, 90, 1000))).toBe(false);
        let rows = append([], sample(1000, 10));
        rows = append(rows, sample(7000, 90, 1000));
        expect(blanks(rows).map((r) => r.ts)).toEqual([1001, 6999]);
    });

    it("breaks for an outage however long its window: a sleeping machine is not a late tick", () => {
        const gap = MAX_COVERED_GAP_MS + 1000;
        expect(coversGap(sample(1000, 10), sample(1000 + gap, 90, gap))).toBe(false);
    });

    it("breaks for a sample with no window (an older srv)", () => {
        const old: DataItem = { ts: 7000, cpu: 90 };
        expect(coversGap(sample(1000, 10), old)).toBe(false);
    });
});

describe("a backfill", () => {
    it("fills a gap the samples are in srv's history for, and the break goes", () => {
        let rows = append([], sample(1000, 10));
        rows = append(rows, sample(7000, 90, 1000));
        expect(blanks(rows)).toHaveLength(2);
        const history = [2000, 3000, 4000, 5000, 6000].map((ts) => sample(ts, 50));
        rows = sampleReducer(rows, { type: "MERGE", items: history, intervalSecs: 1, numPoints: 120 });
        // (The markers at the window's start, before the first sample, are not in the gap.)
        expect(blanks(rows).filter((r) => r.ts > 1000)).toHaveLength(0);
        expect(rows.filter((r) => r.blank !== 1)).toHaveLength(7);
    });

    it("leaves a late tick's step alone: history has nothing for it", () => {
        let rows = append([], sample(1000, 10));
        rows = append(rows, sample(6000, 90, 5000));
        const merged = sampleReducer(rows, {
            type: "MERGE",
            items: [sample(1000, 10), sample(6000, 90, 5000)],
            intervalSecs: 1,
            numPoints: 120,
        });
        expect(merged).toBe(rows);
    });
});
