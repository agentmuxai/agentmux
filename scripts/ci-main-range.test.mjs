// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { LIST_MAX, block, lastVerdictRun, report } from "./ci-main-range.mjs";

const run = (id, at, conclusion, head_sha) => ({ id, created_at: at, conclusion, head_sha });

describe("lastVerdictRun", () => {
    const current = run(9, "2026-11-20T12:00:00Z", "failure", "cccc");
    it("takes the newest earlier run that reached a verdict", () => {
        const runs = [
            run(1, "2026-11-20T08:00:00Z", "success", "aaaa"),
            run(2, "2026-11-20T10:00:00Z", "failure", "bbbb"),
            run(3, "2026-11-20T11:00:00Z", "cancelled", "bbb2"),
            run(4, "2026-11-20T11:30:00Z", null, "bbb3"),
            current,
            run(10, "2026-11-20T13:00:00Z", "success", "dddd"),
        ];
        expect(lastVerdictRun(runs, current).id).toBe(2);
    });
    it("is null with nothing earlier that finished", () => {
        expect(lastVerdictRun([current, run(3, "2026-11-20T11:00:00Z", "cancelled", "x")], current)).toBeNull();
        expect(lastVerdictRun([], current)).toBeNull();
    });
});

describe("block", () => {
    const previous = { head_sha: "aaaaaaaaaaaaaaaa", conclusion: "success" };
    const commits = [
        { sha: "ccccccccccccc", subject: "third" },
        { sha: "bbbbbbbbbbbbb", subject: "second" },
    ];
    it("lists the commits since the last run, newest first", () => {
        const text = block(previous, commits);
        expect(text).toContain("tested `aaaaaaaaa` (success)");
        expect(text).toContain("covers the 2 commits since");
        expect(text.indexOf("`ccccccccc` third")).toBeLessThan(text.indexOf("`bbbbbbbbb` second"));
        expect(text).toContain("one of them or in their combination");
    });
    it("says nothing when the run's own commit is the only one", () => {
        expect(block(previous, [commits[0]])).toBe("");
        expect(block(previous, [])).toBe("");
        expect(block(null, commits)).toBe("");
    });
    it("cuts a long list and still counts the rest", () => {
        const many = Array.from({ length: LIST_MAX + 5 }, (_, i) => ({ sha: `${String(i).padStart(12, "0")}`, subject: `s${i}` }));
        const text = block(previous, many);
        expect(text).toContain(`covers the ${LIST_MAX + 5} commits`);
        expect(text.split("\n").filter((l) => l.startsWith("- `"))).toHaveLength(LIST_MAX);
        expect(text).toContain("- and 5 older");
    });
});

describe("report", () => {
    const env = { REPO: "o/r", SHA: "dddd0000000000", RUN_ID: "9" };
    const gh = (compared) => (args) => {
        const path = args[1];
        if (path.endsWith("/actions/runs/9")) return JSON.stringify(run(9, "2026-11-20T12:00:00Z", "failure", env.SHA));
        if (path.includes("/workflows/ci-pr.yml/runs")) {
            return JSON.stringify({ workflow_runs: [run(2, "2026-11-20T10:00:00Z", "success", "aaaa00000000"), run(9, "2026-11-20T12:00:00Z", "failure", env.SHA)] });
        }
        if (path.includes("/compare/aaaa00000000...dddd0000000000")) return JSON.stringify(compared);
        throw new Error(`unexpected ${args.join(" ")}`);
    };
    it("lists what the last finished run did not cover, oldest first from the API and newest first in the text", () => {
        const text = report(env, gh({ total_commits: 2, commits: [{ sha: "bbbb00000000", commit: { message: "first\n\nbody" } }, { sha: env.SHA, commit: { message: "second" } }] }));
        expect(text.indexOf("second")).toBeLessThan(text.indexOf("first"));
        expect(text).not.toContain("body");
    });
    it("prints nothing for the very next commit", () => {
        expect(report(env, gh({ total_commits: 1, commits: [{ sha: env.SHA, commit: { message: "only" } }] }))).toBe("");
    });
});
