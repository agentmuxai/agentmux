// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { outcomeOf, parseAbArgs, planRuns, runArgs } from "./lib/abplan.mjs";

describe("an interleaved A/B run", () => {
    const base = ["--portable", "P", "--runs", "3", "--out", "out"];

    it("alternates the sides, A first or B first", () => {
        expect(planRuns(parseAbArgs(base)).map((r) => r.label)).toEqual(["a1", "b1", "a2", "b2", "a3", "b3"]);
        expect(planRuns(parseAbArgs([...base, "--order", "ba"])).map((r) => r.label)).toEqual(["b1", "a1", "b2", "a2", "b3", "a3"]);
        expect(planRuns(parseAbArgs([...base, "--a-label", "main", "--b-label", "fix", "--runs", "1"])).map((r) => r.label)).toEqual(["main1", "fix1"]);
    });

    it("gives each run its side's portable, srv and environment, and both sides the same flags", () => {
        const spec = parseAbArgs([...base, "--a-srv", "A.exe", "--a-srv-commit", "aaa", "--b-srv", "B.exe", "--b-env", "AGENTMUX_EXP=on", "--", "--panes", "12", "--disk-load", "50"]);
        const [a1, b1] = planRuns(spec);
        expect(runArgs(a1, spec)).toEqual(["--portable", "P", "--label", "a1", "--out", "out", "--srv", "A.exe", "--srv-commit", "aaa", "--panes", "12", "--disk-load", "50"]);
        expect(runArgs(b1, spec)).toEqual(["--portable", "P", "--label", "b1", "--out", "out", "--srv", "B.exe", "--env", "AGENTMUX_EXP=on", "--panes", "12", "--disk-load", "50"]);
    });

    it("takes a portable per side", () => {
        const spec = parseAbArgs(["--a-portable", "PA", "--b-portable", "PB", "--runs", "2"]);
        expect(runArgs(planRuns(spec)[0], spec).slice(0, 2)).toEqual(["--portable", "PA"]);
        expect(runArgs(planRuns(spec)[1], spec).slice(0, 2)).toEqual(["--portable", "PB"]);
    });

    it("refuses what would make the two sides differ by accident", () => {
        expect(() => parseAbArgs(["--runs", "2"])).toThrow(/--portable/);
        expect(() => parseAbArgs([...base, "--runs", "0"])).toThrow(/whole number/);
        expect(() => parseAbArgs([...base, "--order", "xy"])).toThrow(/--order/);
        expect(() => parseAbArgs([...base, "--a-label", "x", "--b-label", "x"])).toThrow(/different labels/);
        expect(() => parseAbArgs([...base, "--", "--label", "q"])).toThrow(/per side/);
        expect(() => parseAbArgs([...base, "--", "--srv", "X"])).toThrow(/per side/);
        expect(() => parseAbArgs([...base, "--panes", "12"])).toThrow(/after a lone/);
    });

    it("says which runs finished and whether the sides can be compared", () => {
        const plan = planRuns(parseAbArgs(base));
        expect(outcomeOf(plan, new Set(["a1", "b1", "a2", "b3"]))).toEqual({ done: { a: ["a1", "a2"], b: ["b1", "b3"] }, comparable: true });
        expect(outcomeOf(plan, new Set(["a1", "a2", "a3"])).comparable).toBe(false);
    });
});
