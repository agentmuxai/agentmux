// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    formatLong,
    formatSpan,
    latestPages,
    pageWindow,
    pointsOf,
    recordQuery,
    restartsIn,
    type RollupLine,
    stallCounts,
} from "./tower-record-data";

const H = 3_600_000;
const MB = 1024 * 1024;

function roll(h: number, over: Partial<RollupLine> = {}): RollupLine {
    return { h, n: 12, v: "0.60.33", up_s: 100_000, restarts: 0, parts: [], ...over };
}

describe("the record's lines turned into a chart", () => {
    it("charts each hour's median and skips an hour that did not name the figure", () => {
        const lines = [
            roll(0, { am: [1, 100 * MB, 120 * MB, 130 * MB], parts: [["core", [1, 40 * MB, 50 * MB, 60 * MB]]] }),
            roll(H, { parts: [["core", [1, 41 * MB, 50 * MB, 60 * MB]]] }),
            roll(2 * H, { am: [1, 110 * MB, 120 * MB, 130 * MB] }),
        ];
        expect(pointsOf(lines, "am")).toEqual([
            { t: 0, mem: 100 * MB },
            { t: 2 * H, mem: 110 * MB },
        ]);
        expect(pointsOf(lines, "core").map((p) => p.mem)).toEqual([40 * MB, 41 * MB]);
    });

    it("follows the page of the window the newest hour with pages ranks first", () => {
        const page = (w: string, heap: number) => ({ w, heap: [1, heap, heap, heap] as [number, number, number, number], dom: [1, 5_000, 6_000, 7_000] as [number, number, number, number], rows: [0, 300, 400, 500] as [number, number, number, number] });
        const lines = [
            roll(0, { pages: [page("window-3", 200), page("main", 100)] }),
            roll(H, { pages: [page("main", 310), page("window-3", 150)] }),
            roll(2 * H),
        ];
        expect(pageWindow(lines)).toBe("main");
        expect(pointsOf(lines, "heap")).toEqual([
            { t: 0, mem: 100 },
            { t: H, mem: 310 },
        ]);
        expect(pointsOf(lines, "dom").length).toBe(2);
        expect(latestPages(lines).map((p) => p.w)).toEqual(["main", "window-3"]);
        expect(pageWindow([roll(0)])).toBeUndefined();
        expect(latestPages([roll(0)])).toEqual([]);
    });

    it("reads a figure by its kind", () => {
        expect(formatLong("heap", 310.4)).toBe("310 MB");
        expect(formatLong("rows", 4100)).toBe((4100).toLocaleString());
        expect(formatLong("am", 1024 * MB)).toBe("1.0 GB");
        expect(formatLong("am", undefined)).toBe("—");
    });
});

describe("restarts over the lines", () => {
    it("counts the ones inside an hour and the ones between two hours, once each", () => {
        const base = 1_000 * H;
        const lines = [
            roll(base, { up_s: 100_000, last_t: base + H - 60_000 }),
            // Uptime went from 100 000 s to 500 s over an hour: srv started again between the two hours.
            roll(base + H, { up_s: 500, last_t: base + 2 * H - 60_000 }),
            // An hour that counted its own restart is not counted a second time for its short uptime.
            roll(base + 2 * H, { up_s: 300, restarts: 1, last_t: base + 3 * H - 60_000 }),
            roll(base + 3 * H, { up_s: 300 + 3_600, last_t: base + 4 * H - 60_000 }),
        ];
        expect(restartsIn(lines)).toBe(2);
    });

    it("gives an older rollup without a last time 15 minutes of room, not two", () => {
        const base = 1_000 * H;
        const steady = [roll(base, { up_s: 1_000 }), roll(base + H, { up_s: 1_000 + 3_600 - 600 })];
        expect(restartsIn(steady)).toBe(0);
        const exact = [
            roll(base, { up_s: 1_000, last_t: base + H }),
            roll(base + H, { up_s: 1_000 + 3_600 - 600, last_t: base + 2 * H }),
        ];
        expect(restartsIn(exact)).toBe(1);
    });
});

describe("stalls", () => {
    it("counts each kind and its longest, and says when the answer was cut", () => {
        const lines = [
            { t: 1, kind: "ui_stall", ms: 55_000 },
            { t: 2, kind: "ui_stall", ms: 8_000 },
            { t: 3, kind: "srv_stall", ms: 4_000 },
        ];
        const whole = stallCounts({ matched: 3, lines });
        expect(whole.ui).toEqual({ count: 2, longestMs: 55_000 });
        expect(whole.srv).toEqual({ count: 1, longestMs: 4_000 });
        expect(whole.cut).toBe(false);
        expect(stallCounts({ matched: 900, lines }).cut).toBe(true);
        expect(stallCounts({ matched: 0, lines: [] }).ui).toEqual({ count: 0, longestMs: 0 });
    });

    it("writes a span in the unit that reads", () => {
        expect(formatSpan(55_000)).toBe("55 s");
        expect(formatSpan(4 * 60_000)).toBe("4 min");
        expect(formatSpan(65 * 60_000)).toBe("1 h 5 min");
        expect(formatSpan(2 * H)).toBe("2 h");
    });
});

describe("the query", () => {
    it("asks for the span's hours plus two, and for the stalls' newest 500", () => {
        const now = 40 * 86_400_000;
        expect(recordQuery("rollup", "7d", now)).toBe(`kind=rollup&from_ms=${now - 7 * 86_400_000}&limit=170`);
        expect(recordQuery("events", "1d", now)).toBe(`kind=events&from_ms=${now - 86_400_000}&limit=500`);
        expect(recordQuery("rollup", "30d", 1_000)).toContain("from_ms=0&");
    });
});
