// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests for scripts/benchmarks/agent-open.mjs's trace parsing and budget check.
// The CDP driving itself needs a running instance and isn't tested here.

import { describe, expect, it } from "vitest";
import { assess, exitCode, overBudget, parseTrace, phasesOf } from "./agent-open.mjs";

// A real line from a v0.59.11 open (cef-debug.log), as the console reports it.
const LINE =
    '[agent-open] agent="AgentA" block=db9c3a72 source=my-agents outcome=quiet total=2703 cli=53 cli_source=local_install ' +
    "config=87 committed=173 history_start=126 history_read=362 history_lines=1938 parsed=379 painted=854 revealed=937 quiet=1203 auth=verifying";

describe("agent-open benchmark", () => {
    it("parses an [agent-open] line, quoted values included", () => {
        const t = parseTrace(LINE);
        expect(t.agent).toBe("AgentA");
        expect(t.source).toBe("my-agents");
        expect(t.history_read).toBe("362");
    });

    it("measures the read from history_start, not from the click", () => {
        expect(phasesOf(parseTrace(LINE))).toEqual({ read: 236, lines: 1938, painted: 854, revealed: 937, total: 2703, outcome: "quiet" });
    });

    it("has no phases for a trace without a history read", () => {
        expect(phasesOf(parseTrace('[agent-open] agent="X" source=my-agents outcome=closed total=10'))).toBeNull();
    });

    it("names each budget an open misses, and none when within all", () => {
        const args = { readBudget: 1000, revealBudget: 1500, paintBudget: null };
        expect(overBudget({ read: 236, painted: 854, revealed: 937 }, args)).toEqual([]);
        // The v0.59.10 stall: a 17 s read behind the cover.
        expect(overBudget({ read: 17108, painted: 17500, revealed: 17629 }, args)).toEqual(["read 17108 > 1000", "revealed 17629 > 1500"]);
        expect(overBudget({ read: 236, painted: 854, revealed: 937 }, { ...args, paintBudget: 300 })).toEqual(["painted 854 > 300"]);
    });

    it("fails an open that didn't complete, had no read, or never revealed", () => {
        const args = { readBudget: 1000, revealBudget: 1500, paintBudget: null };
        expect(assess(parseTrace(LINE), args)).toEqual([]);
        // A launch that failed before any history read: a failure, not a skipped row.
        expect(assess(parseTrace('[agent-open] agent="X" source=my-agents outcome=failed total=900'), args)).toEqual([
            "outcome=failed",
            "no history read in the trace",
        ]);
        // Read, then closed before the cover lifted.
        expect(assess(parseTrace('[agent-open] agent="X" source=my-agents outcome=closed history_start=100 history_read=300 total=400'), args)).toEqual([
            "outcome=closed",
            "never revealed",
        ]);
        expect(assess(parseTrace(LINE.replace("outcome=quiet", "outcome=timeout")), args)).toEqual(["outcome=timeout"]);
        expect(assess(parseTrace(LINE.replace("outcome=quiet", "outcome=unsettled")), args)).toEqual([]);
    });

    it("fails a stalled open, and treats an agent missing from My Agents as a setup error", () => {
        const ok = { name: "A", status: "traced", misses: [] };
        const alreadyOpen = { name: "B", status: "skipped (already open)" };
        const stalled = { name: "C", status: "stalled", misses: ["no trace within 30000 ms"] };
        const missing = { name: "Typo", status: "not in My Agents" };
        expect(exitCode([ok, alreadyOpen])).toBe(0);
        // One healthy open beside a stalled one still fails the run.
        expect(exitCode([ok, stalled])).toBe(1);
        expect(exitCode([stalled])).toBe(1);
        expect(exitCode([{ ...ok, misses: ["outcome=failed"] }])).toBe(1);
        // A run that covered fewer agents than asked doesn't pass.
        expect(exitCode([ok, missing])).toBe(2);
        expect(exitCode([alreadyOpen])).toBe(2);
    });
});
