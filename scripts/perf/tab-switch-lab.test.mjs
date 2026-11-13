// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { summarizeLongTasks, summarizePane, tableOf } from "./tab-switch-lab.mjs";

const frame = (t, o = {}) => ({ t, vis: true, cov: false, fol: "following", st: 900, sh: 1500, ch: 600, rows: [["a", 100], ["b", 300]], ...o });

describe("summarizePane", () => {
    it("says a pane that never became visible was not shown", () => {
        expect(summarizePane([null, frame(0, { vis: false })])).toEqual({ shown: false });
    });

    it("counts the cover as time with nothing painted, and judges motion between painted frames only", () => {
        const s = summarizePane([
            frame(0, { vis: true, cov: true }),
            frame(16, { cov: true, rows: [["a", 50]] }),
            frame(33, { rows: [["a", 100]] }),
            frame(50, { rows: [["a", 100]] }),
        ]);
        expect(s.coverMs).toBe(16);
        expect(s.blankFrames).toBe(2);
        expect(s.movedFrames).toBe(0);
    });

    it("counts a row that moved on screen, when it last did, and a pane short of its bottom while following", () => {
        const s = summarizePane([
            frame(0),
            frame(16, { rows: [["a", 100], ["b", 340]] }),
            frame(33, { st: 800, rows: [["a", 90], ["b", 340]] }),
            frame(50, { sh: 1700, rows: [["a", 90], ["b", 340]] }),
        ]);
        expect(s.movedFrames).toBe(2);
        expect(s.movedPx).toBe(50);
        expect(s.settledAtMs).toBe(33);
        expect(s.followGapFrames).toBe(2);
        expect(s.maxFollowGapPx).toBe(200);
        expect(s.heightSteps).toBe(1);
        expect(s.heightStepPx).toBe(200);
    });
});

describe("a view that is replaced", () => {
    it("counts a frame with no row in common with the last painted frame, and one painted far short of the bottom while following", () => {
        const s = summarizePane([
            frame(0),
            frame(16, { st: 0, sh: 3200, rows: [["old1", 100], ["old2", 300]] }),
            frame(33, { st: 2600, sh: 3200 }),
        ]);
        expect(s.jumpFrames).toBe(2);
        expect(s.bigGapFrames).toBe(1);
        expect(s.firstBigGapAtMs).toBe(16);
        expect(s.startedVisible).toBe(true);
    });
});

describe("the tables", () => {
    it("sums long tasks", () => {
        expect(summarizeLongTasks([{ at: 0, ms: 60 }, { at: 90, ms: 120 }])).toEqual({ count: 2, totalMs: 180, worstMs: 120 });
    });

    it("takes the median over the repeats", () => {
        const run = (cover, moved) => ({ panes: [{ shown: true, coverMs: cover, movedFrames: moved, movedPx: moved * 10, settledAtMs: 100, heightSteps: 0 }], longTasks: { totalMs: 5 } });
        const [row] = tableOf({ before: { A: [run(300, 4), run(100, 2), run(200, 3)] } });
        expect(row).toMatchObject({ variant: "before", case: "A", runs: 3, coverMs: 200, movedFrames: 3, movedPx: 30 });
    });
});
