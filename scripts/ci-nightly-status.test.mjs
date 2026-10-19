// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for ci-nightly-status.mjs: the nightly-red issue.

import { describe, expect, it } from "vitest";
import { decide, failedTests, finishedJobs, note, report, timedOut, verdict } from "./ci-nightly-status.mjs";

const job = (name, conclusion, status = "completed") => ({ id: 1, name, status, conclusion });

describe("verdict", () => {
    it("is red when the continue-on-error macOS leg failed, though the run shows green", () => {
        // 2026-10-13 to 10-17: the run concluded success with this job red.
        const jobs = [job("cargo test (ubuntu-latest)", "success"), job("cargo test (macos-latest)", "failure"), job("vitest", "success")];
        expect(verdict(jobs)).toBe("red");
    });

    it("is red when a job timed out", () => {
        expect(verdict([job("cargo test (macos-latest)", "timed_out")])).toBe("red");
    });

    it("is green when every job passed or was skipped", () => {
        expect(verdict([job("cargo test (windows-latest)", "success"), job("x", "skipped")])).toBe("green");
    });

    it("says nothing for a cancelled run", () => {
        expect(verdict([job("cargo test (windows-latest)", "success"), job("cargo test (macos-latest)", "cancelled")])).toBe("unknown");
    });

    it("says nothing when no job finished", () => {
        expect(verdict([])).toBe("unknown");
    });
});

describe("finishedJobs", () => {
    it("leaves out the doc link report and jobs still running, such as this one", () => {
        const jobs = [job("Doc link report (non-blocking)", "failure"), job("nightly status", null, "in_progress"), job("vitest", "success")];
        expect(finishedJobs(jobs).map((j) => j.name)).toEqual(["vitest"]);
    });
});

describe("decide", () => {
    it("opens an issue on the first red night", () => {
        expect(decide("red", [])).toEqual({ action: "create" });
    });

    it("comments on the open issue while the streak lasts, the oldest if there are several", () => {
        expect(decide("red", [512, 498])).toEqual({ action: "comment", issues: [498] });
    });

    it("closes every open issue on a green night", () => {
        expect(decide("green", [498, 512])).toEqual({ action: "close", issues: [498, 512] });
    });

    it("does nothing on a green night with no open issue, or an unknown one", () => {
        expect(decide("green", [])).toEqual({ action: "none" });
        expect(decide("unknown", [498])).toEqual({ action: "none" });
    });
});

describe("failedTests", () => {
    it("names failing cargo tests from a raw job log, timestamps stripped", () => {
        const log = [
            "2026-10-18T19:26:13.9198420Z test serve_measures_a_sessions_shell_through_the_daemon ... FAILED",
            "2026-10-18T19:26:13.9200000Z test serve_lists_sessions ... ok",
            "2026-10-18T19:26:18.6185920Z ---- serve_measures_a_sessions_shell_through_the_daemon stdout ----",
        ].join("\n");
        expect(failedTests(log)).toEqual(["serve_measures_a_sessions_shell_through_the_daemon"]);
    });

    it("names a crate that didn't compile, through cargo's colour codes", () => {
        // The 2026-10-18 nightly's log, verbatim.
        const line = "2026-10-18T12:03:20.1799274Z \u001b[1m\u001b[91merror\u001b[0m: could not compile `agentmux-cef` (lib) due to 3 previous errors";
        expect(failedTests(line)).toEqual(["agentmux-cef (did not compile)"]);
    });

    it("names failing vitest files", () => {
        expect(failedTests("2026-10-18T12:00:00.0Z  FAIL  frontend/app/store/recycle.test.ts > recycleWindow > abandons")).toEqual([
            "frontend/app/store/recycle.test.ts > recycleWindow > abandons",
        ]);
    });

    it("lists each test once and stops at the cap", () => {
        const log = Array.from({ length: 30 }, (_, i) => `test t${i % 15} ... FAILED`).join("\n");
        expect(failedTests(log, 10)).toEqual(Array.from({ length: 10 }, (_, i) => `t${i}`));
    });
});

describe("note", () => {
    it("lists each failing job with its tests, and says when one timed out", () => {
        const body = note({
            sha: "71a3e2300abcdef",
            subject: "fix: something",
            runUrl: "https://example.test/run/1",
            failing: [
                { name: "cargo test (macos-latest)", conclusion: "failure", tests: ["a_test"] },
                { name: "cargo release (macos-latest)", conclusion: "timed_out", tests: [] },
            ],
        });
        expect(body).toContain("`71a3e2300` (fix: something): https://example.test/run/1");
        expect(body).toContain("- cargo test (macos-latest)\n  - `a_test`");
        expect(body).toContain("- cargo release (macos-latest) (timed out)");
    });
});

describe("report", () => {
    const env = { REPO: "o/r", RUN_ID: "7", RUN_URL: "https://example.test/run/7", REF: "refs/heads/main", SHA: "71a3e2300abcdef" };
    /** A fake gh: answers the reads from `jobs` and `open`, and records every other call. */
    function fakeGh(jobs, open, annotations = []) {
        const calls = [];
        const gh = (args) => {
            calls.push(args);
            const [cmd, sub] = args;
            if (cmd === "api" && args.includes("--paginate")) return JSON.stringify([{ jobs }]);
            if (cmd === "issue" && sub === "list") return JSON.stringify(open.map((number) => ({ number })));
            if (cmd === "api" && sub.endsWith("/annotations")) return JSON.stringify(annotations);
            if (cmd === "api" && sub.endsWith("/logs")) return "2026-10-18T19:26:13Z test a_test ... FAILED";
            if (cmd === "api" && sub.startsWith("repos/o/r/commits/")) return "fix: something";
            return "";
        };
        return { gh, calls };
    }
    const quiet = () => {};

    it("opens the issue for a red macOS leg, naming the failing test", () => {
        const { gh } = fakeGh([{ ...job("cargo test (macos-latest)", "failure"), id: 42 }, job("vitest", "success")], []);
        const writes = report(env, gh, quiet);
        expect(writes.map((w) => w.slice(0, 2))).toEqual([["label", "create"], ["issue", "create"]]);
        const body = writes[1][writes[1].indexOf("--body") + 1];
        expect(body).toContain("- cargo test (macos-latest)\n  - `a_test`");
    });

    it("reports a job that hit its timeout, which GitHub calls cancelled", () => {
        // The 2026-10-19 nightly: the macOS test job ran out of its 60 minutes saving its cache.
        const { gh } = fakeGh([{ ...job("cargo test (macos-latest)", "cancelled"), id: 9 }], [], [{ annotation_level: "failure", message: "The job has exceeded the maximum execution time of 1h0m0s" }]);
        const writes = report(env, gh, quiet);
        expect(writes.map((w) => w.slice(0, 2))).toEqual([["label", "create"], ["issue", "create"]]);
        expect(writes[1][writes[1].indexOf("--body") + 1]).toContain("- cargo test (macos-latest) (timed out)");
    });

    it("stays silent for a job someone cancelled", () => {
        const { gh } = fakeGh([job("cargo test (macos-latest)", "cancelled")], [498], [{ annotation_level: "failure", message: "The operation was canceled." }]);
        expect(report(env, gh, quiet)).toEqual([]);
    });

    it("comments on the open issue the next red night", () => {
        const { gh } = fakeGh([job("cargo test (macos-latest)", "failure")], [498]);
        expect(report(env, gh, quiet).map((w) => w.slice(0, 3))).toEqual([["issue", "comment", "498"]]);
    });

    it("closes it on a green night", () => {
        const { gh } = fakeGh([job("cargo test (macos-latest)", "success")], [498]);
        expect(report(env, gh, quiet).map((w) => w.slice(0, 3))).toEqual([["issue", "close", "498"]]);
    });

    it("writes nothing in a dry run, but returns what it would write", () => {
        const { gh, calls } = fakeGh([job("cargo test (macos-latest)", "success")], [498]);
        expect(report({ ...env, DRY_RUN: "1" }, gh, quiet)).toHaveLength(1);
        expect(calls.some((c) => c[0] === "issue" && c[1] === "close")).toBe(false);
    });

    it("does nothing for a run on another branch", () => {
        const { gh, calls } = fakeGh([job("x", "failure")], []);
        expect(report({ ...env, REF: "refs/heads/camper/x" }, gh, quiet)).toEqual([]);
        expect(calls).toEqual([]);
    });
});

describe("timedOut", () => {
    it("reads GitHub's timeout annotation, and nothing else", () => {
        expect(timedOut([{ message: "The job has exceeded the maximum execution time of 1h30m0s" }])).toBe(true);
        expect(timedOut([{ message: "The operation was canceled." }, { message: "Node.js 20 is deprecated." }])).toBe(false);
        expect(timedOut([])).toBe(false);
    });
});
