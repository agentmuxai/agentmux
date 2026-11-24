// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { summarizeClick, tableOf, topSelf } from "./minimize-click-lab.mjs";

describe("summarizeClick", () => {
    const click = { name: "click", start: 1000, procStart: 1180, procEnd: 1200, dur: 280 };

    it("splits the click's Event Timing entry into input delay, processing and presentation", () => {
        const s = summarizeClick({ events: [click], longTasks: [], frames: [], downAt: 1000, changedAt: 1260 });
        expect(s).toMatchObject({ seen: true, inputDelay: 180, processing: 20, presentation: 80, clickMs: 280, changedAfter: 260 });
    });

    it("takes the slowest click entry when the page reported several", () => {
        const fast = { name: "click", start: 5000, procStart: 5001, procEnd: 5002, dur: 16 };
        expect(summarizeClick({ events: [fast, click], downAt: 1000 }).inputDelay).toBe(180);
    });

    it("counts only the long tasks that end after the click, and the worst frame gap after it", () => {
        const s = summarizeClick({
            events: [click],
            longTasks: [{ start: 100, ms: 60 }, { start: 1010, ms: 150 }, { start: 1300, ms: 90 }],
            frames: [900, 916, 1000, 1170, 1186],
            downAt: 1000,
            changedAt: null,
        });
        expect(s.longTaskCount).toBe(2);
        expect(s.longTaskMs).toBe(240);
        expect(s.worstFrameMs).toBe(170);
        expect(s.changedAfter).toBeNull();
    });

    it("says nothing was seen when the page reported no click", () => {
        expect(summarizeClick({ events: [], longTasks: [], frames: [], downAt: null })).toMatchObject({ seen: false, inputDelay: null, changedAfter: null });
    });
});

describe("topSelf", () => {
    const profile = {
        nodes: [
            { id: 1, callFrame: { functionName: "(root)", url: "" } },
            { id: 2, callFrame: { functionName: "updateTree", url: "http://x/layout/lib/layoutGeometry.ts", lineNumber: 9 } },
            { id: 3, callFrame: { functionName: "(idle)", url: "" } },
            { id: 4, callFrame: { functionName: "", url: "http://x/app/pane.tsx", lineNumber: 0 } },
        ],
        samples: [2, 2, 3, 4, 2],
        timeDeltas: [1000, 2000, 50000, 500, 1500],
    };

    it("adds a function's self time in ms, leaves idle out and names an anonymous function by its place", () => {
        expect(topSelf(profile, 5)).toEqual([
            { fn: "updateTree lib/layoutGeometry.ts:10", ms: 4.5 },
            { fn: "(anonymous) app/pane.tsx:1", ms: 0.5 },
        ]);
    });

    it("keeps only the n largest and copes with no profile", () => {
        expect(topSelf(profile, 1)).toHaveLength(1);
        expect(topSelf(undefined)).toEqual([]);
    });
});

describe("tableOf", () => {
    it("takes the median of the runs that saw a click, per variant and case", () => {
        const run = (inputDelay, seen = true) => ({ click: { seen, inputDelay, processing: 10, presentation: 20, clickMs: 200, changedAfter: 150, longTaskMs: 0, worstFrameMs: 40 } });
        const rows = tableOf({ before: { "chip-busy": [run(300), run(100), run(200), run(null, false)] }, after: { "chip-busy": [run(50)] } });
        expect(rows[0]).toMatchObject({ variant: "before", case: "chip-busy", runs: "3/4", inputDelay: 200 });
        expect(rows[1]).toMatchObject({ variant: "after", runs: "1/1", inputDelay: 50 });
    });
});
