// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for ci-main-stalled.mjs: a stalled job on main is reported to the
// standing ci-stalled issue, a failed one is not.

import { existsSync, mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { decide, kindOf, LABEL, note, report, stoppedStep, timedOut, TITLE } from "./ci-main-stalled.mjs";

const job = (name, conclusion, extra = {}) => ({ id: 1, name, status: "completed", conclusion, steps: [], ...extra });

describe("kindOf", () => {
    it("is stalled when the only job that did not pass was cancelled (run 43954169035)", () => {
        const jobs = [job("check --tests + test (ubuntu-latest)", "cancelled"), job("vitest", "success"), job("CI required", "failure")];
        expect(kindOf(jobs)).toBe("stalled");
    });

    it("is stalled when the only one timed out", () => {
        expect(kindOf([job("a", "timed_out"), job("b", "success")])).toBe("stalled");
    });

    it("is real when a job failed a step", () => {
        expect(kindOf([job("a", "failure"), job("b", "success"), job("CI required", "failure")])).toBe("real");
    });

    it("is real when a failure sits beside a stalled job: a code failure is not hidden by a stall", () => {
        expect(kindOf([job("a", "cancelled"), job("b", "failure")])).toBe("real");
    });

    it("is real for a job that never started", () => {
        expect(kindOf([job("a", "startup_failure")])).toBe("real");
    });

    it("is none when everything passed or was skipped", () => {
        expect(kindOf([job("a", "success"), job("b", "skipped"), job("CI required", "failure")])).toBe("none");
    });

    it("ignores a job still running and the two jobs of the gate itself", () => {
        const jobs = [job("main status", "cancelled"), job("CI required", "cancelled"), { id: 2, name: "x", status: "in_progress", conclusion: null }, job("ok", "success")];
        expect(kindOf(jobs)).toBe("none");
    });
});

describe("timedOut and stoppedStep", () => {
    it("reads the timeout out of the annotations", () => {
        expect(timedOut([{ message: "The job has exceeded the maximum execution time of 50m0s" }])).toBe(true);
        expect(timedOut([{ message: "The operation was canceled." }, {}])).toBe(false);
    });

    it("names the step that was cancelled, else the first not finished", () => {
        const steps = [
            { name: "build", status: "completed", conclusion: "success" },
            { name: "cargo nextest", status: "completed", conclusion: "cancelled" },
            { name: "Upload", status: "completed", conclusion: "skipped" },
        ];
        expect(stoppedStep(steps)).toBe("cargo nextest");
        expect(stoppedStep([{ name: "a", status: "completed", conclusion: "success" }, { name: "b", status: "in_progress", conclusion: null }])).toBe("b");
        expect(stoppedStep([])).toBeNull();
    });
});

describe("decide", () => {
    it("creates the issue once", () => {
        expect(decide([])).toEqual({ action: "create" });
    });

    it("comments on the open one, the oldest if there are several", () => {
        expect(decide([{ number: 9, state: "OPEN" }, { number: 4, state: "OPEN" }])).toEqual({ action: "comment", issue: 4 });
    });

    it("reopens a closed one rather than opening another", () => {
        expect(decide([{ number: 4, state: "CLOSED" }])).toEqual({ action: "reopen", issue: 4 });
    });
});

describe("note", () => {
    const stalled = [{ name: "check --tests + test (ubuntu-latest)", timedOut: true, step: "cargo nextest (CEF-free crates)", warnings: ["The nextest step stalled (status 124 after at most 1500 s)"] }];

    it("names the run, the job, the step and the trace", () => {
        const text = note({ sha: "95dd4691bacf", subject: "x (#1)", runUrl: "https://example/run/1", stalled, traces: [{ name: "resource-trace", url: "https://example/run/1/artifacts/7" }] });
        expect(text).toContain("`95dd4691b`");
        expect(text).toContain("https://example/run/1");
        expect(text).toContain('- check --tests + test (ubuntu-latest) (timed out, in the step "cargo nextest (CEF-free crates)")');
        expect(text).toContain("stalled (status 124");
        expect(text).toContain("[resource-trace](https://example/run/1/artifacts/7)");
        expect(text).toContain("Main is not red");
    });

    it("says when no trace was kept", () => {
        const text = note({ sha: "95dd4691bacf", subject: "x", runUrl: "u", stalled, traces: [] });
        expect(text).toContain("none was kept");
    });
});

/** A fake gh over one run, recording what the job asks it. */
function fakeGh({ jobs, annotations = [], artifacts = [], labelled = [], created = "https://github.com/o/r/issues/12" }) {
    const calls = [];
    const gh = (args) => {
        calls.push(args);
        const [a, b] = args;
        const path = args.find((x) => typeof x === "string" && x.startsWith("repos/")) ?? "";
        if (a === "api" && path.includes("/actions/runs/") && path.includes("/jobs")) return JSON.stringify([{ jobs }]);
        if (a === "api" && path.includes("/annotations")) return JSON.stringify([annotations]);
        if (a === "api" && path.includes("/artifacts")) return JSON.stringify({ artifacts });
        if (a === "api" && path.includes("/commits/")) return "the head's subject";
        if (a === "issue" && b === "list") return JSON.stringify(labelled);
        if (a === "issue" && b === "create") return created;
        return "";
    };
    return { gh, calls, writes: () => calls.filter((c) => ["comment", "reopen", "create", "close"].includes(c[1]) || c[0] === "label") };
}

const stalledRun = [
    job("CI required", "failure"),
    job("vitest", "success"),
    job("check --tests + test (ubuntu-latest)", "cancelled", { id: 5, steps: [{ name: "cargo nextest", status: "completed", conclusion: "cancelled" }] }),
];

function env(extra = {}) {
    const dir = mkdtempSync(join(tmpdir(), "stalled-"));
    const outFile = join(dir, "out");
    return { env: { REPO: "o/r", RUN_ID: "1", RUN_URL: "https://x/run/1", SHA: "95dd4691bacf0000", RESULT: "failure", GITHUB_OUTPUT: outFile, ...extra }, outFile };
}

describe("report", () => {
    const quiet = () => {};

    it("creates the standing issue on a stall, whether or not the push is still main's head", () => {
        const { env: e, outFile } = env();
        const f = fakeGh({ jobs: stalledRun, artifacts: [{ id: 7, name: "resource-trace-ubuntu", expired: false }] });
        const writes = report(e, f.gh, quiet);
        expect(readFileSync(outFile, "utf8")).toBe("kind=stalled\n");
        expect(writes.map((w) => w[0] + " " + w[1])).toEqual(["label create", "issue create", "issue comment"]);
        const create = writes[1];
        expect(create[create.indexOf("--title") + 1]).toBe(TITLE);
        expect(create[create.indexOf("--label") + 1]).toBe(LABEL);
        // The issue's own text holds no occurrence: the first occurrence is a comment, like every later one.
        expect(create[create.indexOf("--body") + 1]).not.toContain("artifacts/7");
        const comment = writes[2];
        expect(comment[2]).toBe("12");
        expect(comment[comment.indexOf("--body") + 1]).toContain("artifacts/7");
        // It never asks for main's head: the decision does not depend on it.
        expect(f.calls.some((c) => c.join(" ").includes("git/ref"))).toBe(false);
    });

    it("comments on the open standing issue", () => {
        const { env: e } = env();
        const f = fakeGh({ jobs: stalledRun, labelled: [{ number: 12, state: "OPEN" }] });
        expect(report(e, f.gh, quiet).map((w) => w.slice(0, 3))).toEqual([["issue", "comment", "12"]]);
    });

    it("reopens a closed one", () => {
        const { env: e } = env();
        const f = fakeGh({ jobs: stalledRun, labelled: [{ number: 12, state: "CLOSED" }] });
        expect(report(e, f.gh, quiet).map((w) => w.slice(0, 3))).toEqual([["issue", "reopen", "12"]]);
    });

    it("closes its own as a duplicate when an older one appeared at the same moment", () => {
        const { env: e } = env();
        const f = fakeGh({ jobs: stalledRun, created: "https://github.com/o/r/issues/13" });
        // The first list (before the create) is empty; the second shows the other run's issue 11 beside this one's 13.
        let n = 0;
        const gh = (args) => (args[0] === "issue" && args[1] === "list" ? JSON.stringify(++n === 1 ? [] : [{ number: 11, state: "OPEN" }, { number: 13, state: "OPEN" }]) : f.gh(args));
        const writes = report(e, gh, quiet);
        expect(writes.map((w) => w.slice(0, 3))).toEqual([["label", "create", LABEL], ["issue", "create", "--repo"], ["issue", "comment", "11"], ["issue", "close", "13"]]);
        expect(writes[3].join(" ")).toContain("Duplicate of #11");
    });

    it("leaves kind unset when the report could not be written, so main status still reports the failure as main-red", () => {
        const { env: e, outFile } = env();
        const f = fakeGh({ jobs: stalledRun, labelled: [{ number: 12, state: "OPEN" }] });
        const gh = (args) => {
            if (args[0] === "issue" && args[1] === "comment") throw new Error("HTTP 502");
            return f.gh(args);
        };
        expect(() => report(e, gh, quiet)).toThrow("HTTP 502");
        expect(existsSync(outFile) ? readFileSync(outFile, "utf8") : "").not.toContain("stalled");
    });

    it("still says stalled when only closing a duplicate fails: the occurrence is reported", () => {
        const { env: e, outFile } = env();
        const f = fakeGh({ jobs: stalledRun, created: "https://github.com/o/r/issues/13" });
        let n = 0;
        const gh = (args) => {
            if (args[0] === "issue" && args[1] === "close") throw new Error("HTTP 502");
            if (args[0] === "issue" && args[1] === "list") return JSON.stringify(++n === 1 ? [] : [{ number: 11, state: "OPEN" }, { number: 13, state: "OPEN" }]);
            return f.gh(args);
        };
        report(e, gh, quiet);
        expect(readFileSync(outFile, "utf8")).toBe("kind=stalled\n");
    });

    it("writes the stalled kind after the report, in a dry run too", () => {
        const { env: e, outFile } = env({ DRY_RUN: "1" });
        const f = fakeGh({ jobs: stalledRun });
        const lines = [];
        report(e, f.gh, (l) => lines.push(l));
        expect(lines.findIndex((l) => l.startsWith("[dry run] gh issue comment"))).toBeLessThan(lines.indexOf("kind=stalled"));
        expect(readFileSync(outFile, "utf8")).toBe("kind=stalled\n");
    });

    it("writes nothing for a real failure, so main status reports it as before", () => {
        const { env: e, outFile } = env();
        const f = fakeGh({ jobs: [job("CI required", "failure"), job("vitest", "failure")] });
        expect(report(e, f.gh, quiet)).toEqual([]);
        expect(readFileSync(outFile, "utf8")).toBe("kind=real\n");
    });

    it("writes nothing for a mix of failure and stall", () => {
        const { env: e, outFile } = env();
        const f = fakeGh({ jobs: [job("CI required", "failure"), job("vitest", "failure"), ...stalledRun.slice(2)] });
        expect(report(e, f.gh, quiet)).toEqual([]);
        expect(readFileSync(outFile, "utf8")).toBe("kind=real\n");
    });

    it("says none and asks nothing when CI required did not fail", () => {
        const { env: e, outFile } = env({ RESULT: "success" });
        const f = fakeGh({ jobs: stalledRun });
        expect(report(e, f.gh, quiet)).toEqual([]);
        expect(f.calls).toEqual([]);
        expect(readFileSync(outFile, "utf8")).toBe("kind=none\n");
    });

    it("prints instead of writing in a dry run", () => {
        const { env: e } = env({ DRY_RUN: "1" });
        const f = fakeGh({ jobs: stalledRun });
        const lines = [];
        report(e, (args) => f.gh(args), (l) => lines.push(l));
        expect(lines.some((l) => l.startsWith("[dry run] gh issue create"))).toBe(true);
        expect(f.writes()).toEqual([]);
    });
});
