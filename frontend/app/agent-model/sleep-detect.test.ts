// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect } from "vitest";
import { wholeCommandSleepMs, SLEEP_IMMEDIATE_MIN_MS } from "./sleep-detect";

describe("wholeCommandSleepMs — matches", () => {
    it("a bare sleep in seconds", () => {
        expect(wholeCommandSleepMs("sleep 300")).toBe(300_000);
        expect(wholeCommandSleepMs("sleep 60")).toBe(60_000);
    });

    it("tolerates surrounding whitespace and a trailing semicolon", () => {
        expect(wholeCommandSleepMs("  sleep 300  ")).toBe(300_000);
        expect(wholeCommandSleepMs("sleep 300;")).toBe(300_000);
        expect(wholeCommandSleepMs("sleep 300 ; ")).toBe(300_000);
    });

    it("understands unit suffixes", () => {
        expect(wholeCommandSleepMs("sleep 30s")).toBe(30_000);
        expect(wholeCommandSleepMs("sleep 5m")).toBe(300_000);
        expect(wholeCommandSleepMs("sleep 1h")).toBe(3_600_000);
    });

    it("understands fractional seconds", () => {
        expect(wholeCommandSleepMs("sleep 7.5")).toBe(7_500);
    });

    /** `timeout N sleep N` is a real idiom for a bounded wait and is still
     *  purely a wait — nothing runs after it. */
    it("accepts a timeout-wrapped sleep", () => {
        expect(wholeCommandSleepMs("timeout 60 sleep 60")).toBe(60_000);
        expect(wholeCommandSleepMs("timeout 90s sleep 60")).toBe(60_000);
    });
});

describe("wholeCommandSleepMs — refuses anything with a second clause", () => {
    // These are the 204-of-270 real-transcript cases that made the naive
    // "starts with sleep" rule wrong 76% of the time. Every one is
    // wait-then-do-something, not a pure wait, and every one runs long enough
    // that ordinary duration promotion already catches it.
    it.each([
        "sleep 90; tail -30 /tmp/build.log",
        "sleep 60; ls ~/Documents/out",
        "sleep 30; cat /tmp/task.output",
        "sleep 20 && tail -30 /tmp/x.log",
        "sleep 2 && rm -rf /tmp/staging",
        "sleep 45 cd /repo && npm test",
        "sleep 10 | tee /tmp/log",
        "sleep 10\necho done",
    ])("refuses %j", (cmd) => {
        expect(wholeCommandSleepMs(cmd)).toBeNull();
    });

    it("refuses a loop that merely contains a sleep", () => {
        // The Agent1 heartbeat shape. Genuinely long-running, but duration
        // promotion is what catches it — a text rule can't read intent here,
        // and `sleep 25` inside the loop is not this command's total wait.
        expect(wholeCommandSleepMs('while true; do sleep 25; echo "[hb]"; done')).toBeNull();
    });
});

describe("wholeCommandSleepMs — refuses non-sleeps and malformed input", () => {
    it.each([
        ["a bare sleep with no argument", "sleep"],
        ["a non-numeric argument", "sleep forever"],
        ["GNU multi-arg sleep (summing it isn't worth the surface)", "sleep 1 2"],
        ["a different command entirely", "npm test"],
        ["a command merely mentioning sleep", "grep -r sleep src/"],
        ["a command ending in sleep", "npm run sleep"],
    ])("refuses %s", (_label, cmd) => {
        expect(wholeCommandSleepMs(cmd)).toBeNull();
    });

    it("handles empty/undefined without throwing", () => {
        expect(wholeCommandSleepMs("")).toBeNull();
        expect(wholeCommandSleepMs(undefined)).toBeNull();
    });
});

describe("wholeCommandSleepMs — the micro-delay floor", () => {
    it("refuses a sleep below the floor, so a micro-delay never takes a dock row", () => {
        expect(wholeCommandSleepMs("sleep 1")).toBeNull();
        expect(wholeCommandSleepMs("sleep 2")).toBeNull();
        expect(wholeCommandSleepMs("sleep 4.9")).toBeNull();
    });

    it("accepts exactly at the floor", () => {
        expect(wholeCommandSleepMs("sleep 5")).toBe(SLEEP_IMMEDIATE_MIN_MS);
    });
});

describe("wholeCommandSleepMs — timeout wrapper decides the real wait (codex P2, PR #2851)", () => {
    /** `timeout N sleep M` ends at whichever comes first. Counting down from
     *  the sleep's own argument would say "~300s left" about a process that
     *  dies in 5 — the confidently-wrong reading this module exists to avoid. */
    it("uses the timeout when it is shorter than the sleep", () => {
        expect(wholeCommandSleepMs("timeout 5 sleep 300")).toBe(5_000);
        expect(wholeCommandSleepMs("timeout 30s sleep 600")).toBe(30_000);
        expect(wholeCommandSleepMs("timeout 1m sleep 3600")).toBe(60_000);
    });

    it("uses the sleep when the timeout is longer (it never fires)", () => {
        expect(wholeCommandSleepMs("timeout 600 sleep 30")).toBe(30_000);
    });

    /** `timeout --help`: "A duration of 0 disables the associated timeout." */
    it("treats a zero timeout as no timeout at all", () => {
        expect(wholeCommandSleepMs("timeout 0 sleep 300")).toBe(300_000);
    });

    it("applies the micro-delay floor to the EFFECTIVE wait, not the sleep argument", () => {
        // Dies after 2s despite the `sleep 300` — below the floor, so no row.
        expect(wholeCommandSleepMs("timeout 2 sleep 300")).toBeNull();
    });
});

describe("wholeCommandSleepMs — case sensitivity matches the shell (codex P2, PR #2851)", () => {
    /** Verified against coreutils: `sleep 5M` -> "invalid time interval '5M'".
     *  Matching it would promote a call that fails instantly — phantom dock
     *  row, suppressed working row, bogus attached-task exemption. */
    it("refuses uppercase unit suffixes, which coreutils rejects", () => {
        expect(wholeCommandSleepMs("sleep 5M")).toBeNull();
        expect(wholeCommandSleepMs("sleep 30S")).toBeNull();
        expect(wholeCommandSleepMs("sleep 1H")).toBeNull();
    });

    it("refuses an uppercase command name, which is not a command on a case-sensitive fs", () => {
        expect(wholeCommandSleepMs("SLEEP 60")).toBeNull();
        expect(wholeCommandSleepMs("Sleep 60")).toBeNull();
    });

    it("refuses an uppercase timeout wrapper", () => {
        expect(wholeCommandSleepMs("TIMEOUT 60 sleep 60")).toBeNull();
    });

    it("still accepts the ordinary lowercase forms", () => {
        expect(wholeCommandSleepMs("sleep 5m")).toBe(300_000);
        expect(wholeCommandSleepMs("sleep 30s")).toBe(30_000);
    });
});
