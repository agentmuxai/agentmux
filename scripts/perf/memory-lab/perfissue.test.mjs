// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { closeNote, failedSteps, flaggedMeasures, issueBody, perfState, reportIssue, reportRunFailure, runState, timedOutResult } from "./lib/perfissue.mjs";

const cell = (a, b, changePct, flag) => ({ a, b, change_pct: changePct, flag });
const compare = (measures, head = ["b1", "b2", "b3", "b4"], base = ["a1", "a2", "a3", "a4"]) => ({ base, head, measures, flags: Object.entries(measures).filter(([, m]) => m?.flag).map(([k]) => k) });
const clean = compare({ app_mb: cell(640, 652, 1.9, false), srv_cpu_s: cell(38, 39, 2.6, false), open_first_screen_ms: null });
const worse = compare({ app_mb: cell(640, 652, 1.9, false), srv_cpu_s: cell(38, 44.1, 16.1, true) });
// the same night's comparison over the first matrix and the confirmation pair together: one more run a side
const withPair = (c) => ({ ...c, base: [...c.base, "a1"], head: [...c.head, "b1"] });
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
        expect(perfState({ hasA: true, initial: worse, final: withPair(worse) })).toBe("red");
        // flagged by the first matrix, gone once the confirmation pair is in: one noisy run must not open an issue
        expect(perfState({ hasA: true, initial: worse, final: withPair(clean) })).toBe("green");
        expect(perfState({ hasA: true, initial: clean, final: clean })).toBe("green");
    });

    it("does not call a flag red when the confirmation pair added no runs, or only one side's", () => {
        expect(perfState({ hasA: true, initial: worse, final: worse })).toBe("unknown");
        expect(perfState({ hasA: true, initial: worse, final: { ...worse, head: [...worse.head, "b1"] } })).toBe("unknown"); // B1 finished, A1 did not
    });

    it("is unknown with nothing to compare or when the lab did not write its comparison", () => {
        expect(perfState({ hasA: false, initial: worse, final: withPair(worse) })).toBe("unknown");
        expect(perfState({ hasA: true, initial: null, final: null })).toBe("unknown");
        expect(perfState({ hasA: true, initial: worse, final: null })).toBe("unknown");
    });
});

describe("the issue text", () => {
    it("names the measure, the size of the change, both builds, the merges and the run", () => {
        const body = issueBody({ plan, initial: worse, final: withPair(worse), runUrl: "https://example/run/1" });
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
        const { writes } = run([], { initial: worse, final: withPair(worse) });
        expect(writes.map((w) => w.slice(0, 2).join(" "))).toEqual(["label create", "issue create"]);
        expect(writes[1]).toContain("perf-regression");
    });

    it("comments on the oldest open issue on a later flagged night, and creates nothing", () => {
        const { writes } = run([9, 7], { initial: worse, final: withPair(worse) });
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
        const { writes, calls } = run([], { initial: worse, final: withPair(worse), dry: "1" });
        expect(writes).toHaveLength(2);
        expect(calls.every((c) => c[1] === "list")).toBe(true);
    });
});

describe("a night that did not finish", () => {
    const TIMEOUT = [{ message: "The job has exceeded the maximum execution time of 2h0m0s" }];

    it("is red for a failure, green for success, and says nothing when cancelled or skipped", () => {
        expect([runState("failure"), runState("success"), runState("cancelled"), runState("skipped"), runState(undefined)]).toEqual(["red", "green", "unknown", "unknown", "unknown"]);
    });

    it("reads a cancelled job that hit its timeout as a failure (GitHub reports a timeout as cancelled), and leaves a plain cancel alone", () => {
        const jobs = [{ id: 1, conclusion: "cancelled" }];
        expect(timedOutResult("cancelled", jobs, () => TIMEOUT)).toBe("failure");
        expect(timedOutResult("cancelled", jobs, () => [{ message: "The operation was canceled." }])).toBe("cancelled");
        expect(timedOutResult("cancelled", jobs, () => { throw new Error("no annotations"); })).toBe("cancelled");
        expect(timedOutResult("success", jobs, () => TIMEOUT)).toBe("success");
    });

    it("names the failed steps of the failed job", () => {
        const jobs = [
            { name: "lab", conclusion: "failure", steps: [{ name: "Checkout", conclusion: "success" }, { name: "Run the lab (A and B interleaved)", conclusion: "failure" }] },
            { name: "other", conclusion: "success", steps: [{ name: "x", conclusion: "failure" }] },
            { name: "timed", conclusion: "timed_out", steps: [] },
        ];
        expect(failedSteps(jobs)).toEqual(["lab: Run the lab (A and B interleaved)", "timed"]);
    });

    const run = (result, open, jobs = [], annotations = []) => {
        const calls = [];
        const gh = (a) => {
            calls.push(a);
            if (a[0] === "issue" && a[1] === "list") return JSON.stringify(open.map((number) => ({ number })));
            if (a[0] === "api" && a[3].includes("/jobs")) return JSON.stringify([{ jobs }]);
            if (a[0] === "api" && a[3].includes("/annotations")) return JSON.stringify([annotations]);
            return "";
        };
        const writes = reportRunFailure({ env: { REPO: "o/r", RUN_URL: "u", RUN_ID: "5" }, result, gh, log: () => {} });
        return { writes, calls };
    };

    it("opens one perf-run-failed issue for the first failed night, with the failed step", () => {
        const { writes } = run("failure", [], [{ id: 1, name: "lab", conclusion: "failure", steps: [{ name: "Report", conclusion: "failure" }] }]);
        expect(writes.map((w) => w.slice(0, 2).join(" "))).toEqual(["label create", "issue create"]);
        expect(writes[1]).toContain("perf-run-failed");
        expect(writes[1].join(" ")).toContain("- lab: Report");
    });

    it("opens the issue for a job that hit its timeout, naming the step that was running", () => {
        const jobs = [{ id: 1, name: "lab", conclusion: "cancelled", steps: [{ name: "Checkout", conclusion: "success" }, { name: "Run the lab (A and B interleaved)", conclusion: "cancelled" }] }];
        const { writes } = run("cancelled", [], jobs, TIMEOUT);
        expect(writes.map((w) => w.slice(0, 2).join(" "))).toEqual(["label create", "issue create"]);
        expect(writes[1].join(" ")).toContain("- lab: Run the lab (A and B interleaved)");
    });

    it("comments on the open one for a later failed night, and closes it when a night finishes", () => {
        expect(run("failure", [4, 8]).writes.map((w) => w.slice(0, 3).join(" "))).toEqual(["issue comment 4"]);
        expect(run("success", [4, 8]).writes.map((w) => w.slice(0, 3).join(" "))).toEqual(["issue close 4", "issue close 8"]);
    });

    it("does nothing for a plain cancel, or a finished night with no open issue", () => {
        expect(run("cancelled", [4], [{ id: 1, conclusion: "cancelled" }], [{ message: "The operation was canceled." }]).writes).toEqual([]);
        expect(run("success", []).writes).toEqual([]);
    });
});
