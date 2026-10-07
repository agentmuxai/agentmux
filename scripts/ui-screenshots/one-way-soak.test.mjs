// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { parseArgs, summarize } from "./one-way-soak.mjs";

const stats = (over) => ({
    pane: "abc1234",
    framesChecked: 100,
    framesFollowing: 90,
    down: 0,
    overshoot: 0,
    maxDy: 0,
    blankFrames: 0,
    maxGap: 0,
    byCause: {},
    recent: [],
    ...over,
});

describe("one-way-soak parseArgs", () => {
    it("has dev-safe defaults", () => {
        const o = parseArgs([]);
        expect(o.port).toBe(9223);
        expect(o.panes).toEqual([]);
        expect(o.minutes).toBe(3);
        expect(o.strict).toBe(false);
    });

    it("reads panes, minutes and flags", () => {
        const o = parseArgs(["--panes", "a,b", "--minutes", "30", "--strict", "--keep", "--seed", "7"]);
        expect(o.panes).toEqual(["a", "b"]);
        expect(o.minutes).toBe(30);
        expect(o.seed).toBe(7);
        expect(o.strict).toBe(true);
        expect(o.keep).toBe(true);
    });

    it("rejects a negative number", () => {
        expect(() => parseArgs(["--minutes", "-1"])).toThrow(/non-negative/);
    });
});

describe("one-way-soak summarize", () => {
    it("passes with no violations", () => {
        const r = summarize([stats({})]);
        expect(r.violations).toBe(0);
        expect(r.text).toMatch(/PASS/);
    });

    it("totals violations and ranks causes across panes", () => {
        const r = summarize([
            stats({ down: 2, overshoot: 1, byCause: { "hold:release-ease": 2, transform: 1 } }),
            stats({ pane: "def5678", down: 3, byCause: { transform: 3 } }),
        ]);
        expect(r.violations).toBe(6);
        expect(r.text).toMatch(/causes: transform×4 {2}hold:release-ease×2/);
        expect(r.text).toMatch(/FAIL: 6/);
    });
});
