// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { join } from "node:path";

import { describe, expect, it } from "vitest";

import { compareArgs, outcomeOf, parseAbArgs, planRuns, runArgs, sideGivenUp, unrunnableSides } from "./lib/abplan.mjs";

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

    it("hands the finished runs, the outputs and the margins file to the comparison", () => {
        const spec = parseAbArgs([...base, "--margins", "budget.json"]);
        const outcome = outcomeOf(planRuns(spec), new Set(["a1", "b1", "b2"]));
        expect(compareArgs(spec, outcome)).toEqual([
            "--base", join("out", "a1", "result.json"),
            "--head", [join("out", "b1", "result.json"), join("out", "b2", "result.json")].join(","),
            "--md", join("out", "compare.md"), "--json", join("out", "compare.json"),
            "--margins", "budget.json",
        ]);
        expect(compareArgs(parseAbArgs(base), outcome)).not.toContain("--margins");
    });
});

describe("a side that cannot run", () => {
    const spec = parseAbArgs(["--portable", "P", "--runs", "4", "--out", "out"]);
    const plan = planRuns(spec);
    const failed = (run) => ({ ...run, ok: false });

    it("gives each side the Claude version its own srv pins", () => {
        const pinned = parseAbArgs(["--portable", "P", "--a-claude-version", "2.1.288", "--b-claude-version", "2.1.321"]);
        const [a1, b1] = planRuns(pinned);
        expect(runArgs(a1, pinned)).toContain("2.1.288");
        expect(runArgs(a1, pinned)).not.toContain("2.1.321");
        const bArgs = runArgs(b1, pinned);
        expect(bArgs[bArgs.indexOf("--claude-version") + 1]).toBe("2.1.321");
        expect(runArgs(a1, spec)).not.toContain("--claude-version");
    });

    it("stops trying a side after two failed runs with none finished, and only that side", () => {
        const ran = [failed(plan[0]), { ...plan[1], ok: true }, failed(plan[2])];
        expect(sideGivenUp(plan[4], ran)).toBe(true); // a3
        expect(sideGivenUp(plan[5], ran)).toBe(false); // b3: B finished one
        expect(sideGivenUp(plan[2], [failed(plan[0])])).toBe(false); // one failure is not enough
        expect(sideGivenUp(plan[4], [failed(plan[0]), { ...plan[2], ok: true }])).toBe(false); // A finished one
    });

    it("says in words which side did not run and why, with the build when it is known", () => {
        const done = { a: [], b: ["b1", "b2"] };
        const outcome = { done, comparable: false };
        const reasons = { a1: "12 of 12 agents never played their turn", a2: "the same" };
        expect(unrunnableSides(plan, outcome, reasons, ["a3", "a4"], { a: "a8da0848e0123", b: "b4fa1c400" })).toEqual([
            "A (build a8da0848e) did not run: 12 of 12 agents never played their turn (2 runs tried, 2 not tried after those failed).",
        ]);
        expect(unrunnableSides(plan, { done: { a: ["a1"], b: ["b1"] }, comparable: true }, {})).toEqual([]);
        expect(unrunnableSides(plan, { done: { a: [], b: [] }, comparable: false }, {})).toHaveLength(2);
        expect(unrunnableSides(plan, outcome, {})[0]).toContain("no run wrote a result");
    });
});
