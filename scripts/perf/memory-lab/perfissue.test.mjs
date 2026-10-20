// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { closeNote, flaggedMeasures, issueBody, perfState, reportIssue } from "./lib/perfissue.mjs";

const cell = (a, b, changePct, flag) => ({ a, b, change_pct: changePct, flag });
const compare = (measures) => ({ measures, flags: Object.entries(measures).filter(([, m]) => m?.flag).map(([k]) => k) });
const clean = compare({ app_mb: cell(640, 652, 1.9, false), srv_cpu_s: cell(38, 39, 2.6, false), open_first_screen_ms: null });
const worse = compare({ app_mb: cell(640, 652, 1.9, false), srv_cpu_s: cell(38, 44.1, 16.1, true) });
const plan = {
    a: { id: 111, sha: "71a3e2300aaaa", createdAt: "2026-10-18T11:00:00Z" },
    b: { id: 112, sha: "5ba04f91fbbbb", createdAt: "2026-10-19T11:00:00Z" },
    merges: { total: 2, truncated: false, shown: [{ sha: "abc1234", title: "perf: one (#1)" }, { sha: "def5678", title: "fix: two (#2)" }] },
};

describe("which measures a comparison flags", () => {
    it("lists the flagged ones with their medians and change, and skips measures the lab did not produce", () => {
        expect(flaggedMeasures(worse)).toEqual([{ name: "srv_cpu_s", a: 38, b: 44.1, changePct: 16.1 }]);
        expect(flaggedMeasures(clean)).toEqual([]);
        expect(flaggedMeasures(null)).toEqual([]);
    });
});

describe("what the night says", () => {
    it("is red only when the flag holds over all the runs together", () => {
        expect(perfState({ hasA: true, initial: worse, final: worse })).toBe("red");
        // flagged by the first matrix, gone once the confirmation pair is in: one noisy run must not open an issue
        expect(perfState({ hasA: true, initial: worse, final: clean })).toBe("green");
        expect(perfState({ hasA: true, initial: clean, final: clean })).toBe("green");
    });

    it("is unknown with nothing to compare or when the lab did not write its comparison", () => {
        expect(perfState({ hasA: false, initial: worse, final: worse })).toBe("unknown");
        expect(perfState({ hasA: true, initial: null, final: null })).toBe("unknown");
        expect(perfState({ hasA: true, initial: worse, final: null })).toBe("unknown");
    });
});

describe("the issue text", () => {
    it("names the measure, the size of the change, both builds, the merges and the run", () => {
        const body = issueBody({ plan, initial: worse, final: worse, runUrl: "https://example/run/1" });
        expect(body).toContain("| srv_cpu_s | 38 | 44.1 | +16.1% |");
        expect(body).toContain("`71a3e2300`");
        expect(body).toContain("`5ba04f91f`");
        expect(body).toContain("`abc1234` perf: one (#1)");
        expect(body).toContain("a confirmation pair");
        expect(body).toContain("https://example/run/1");
        expect(body).not.toContain("app_mb");
    });

    it("says how many merges were not shown, and does not mention a confirmation pair when none ran", () => {
        const body = issueBody({ plan: { ...plan, merges: { total: 40, truncated: true, shown: plan.merges.shown } }, initial: clean, final: worse, runUrl: "u" });
        expect(body).toContain("(40, newest 2 shown)");
        expect(body).not.toContain("confirmation pair");
    });

    it("words a drift as a drift", () => {
        expect(issueBody({ plan, initial: worse, final: worse, runUrl: "u", kind: "drift" })).toContain("slow drift");
    });

    it("closes with the two commits", () => {
        expect(closeNote({ plan, runUrl: "u" })).toContain("`71a3e2300` and `5ba04f91f`");
    });
});

describe("the issue step", () => {
    const run = (open, args) => {
        const calls = [];
        const gh = (a) => {
            calls.push(a);
            return a[0] === "issue" && a[1] === "list" ? JSON.stringify(open.map((number) => ({ number }))) : "";
        };
        const writes = reportIssue({ env: { REPO: "o/r", RUN_URL: "u", DRY_RUN: args.dry }, plan, initial: args.initial, final: args.final, gh, log: () => {} });
        return { writes, calls };
    };

    it("creates the label and the issue on the first flagged night", () => {
        const { writes } = run([], { initial: worse, final: worse });
        expect(writes.map((w) => w.slice(0, 2).join(" "))).toEqual(["label create", "issue create"]);
        expect(writes[1]).toContain("perf-regression");
    });

    it("comments on the oldest open issue on a later flagged night, and creates nothing", () => {
        const { writes } = run([9, 7], { initial: worse, final: worse });
        expect(writes).toHaveLength(1);
        expect(writes[0].slice(0, 3)).toEqual(["issue", "comment", "7"]);
    });

    it("closes every open issue on a clean night", () => {
        const { writes } = run([7, 9], { initial: clean, final: clean });
        expect(writes.map((w) => w.slice(0, 3).join(" "))).toEqual(["issue close 7", "issue close 9"]);
    });

    it("does nothing on an unflagged night with no open issue", () => {
        expect(run([], { initial: clean, final: clean }).writes).toEqual([]);
    });

    it("leaves an open issue alone when the lab did not finish", () => {
        expect(run([7], { initial: null, final: null }).writes).toEqual([]);
    });

    it("sends nothing with DRY_RUN, but says what it would have", () => {
        const { writes, calls } = run([], { initial: worse, final: worse, dry: "1" });
        expect(writes).toHaveLength(2);
        expect(calls.every((c) => c[1] === "list")).toBe(true);
    });
});
