#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The glue between the nightly performance workflow (.github/workflows/perf-regression.yml) and GitHub.
//
//   node nightly.mjs plan   [--b-run <id>] --out plan.json   picks the builds to compare, writes plan.json and the step outputs
//   node nightly.mjs report --plan plan.json [--compare compare.md] [--spread spread.md]   the markdown for the job summary
//   node nightly.mjs flagged --compare compare.json   whether the first matrix flagged anything (sets the `flagged` output)
//   node nightly.mjs issue --plan plan.json --initial compare.json --final final.json   opens, comments on or closes the perf-regression issue
//   node nightly.mjs failure --result <success|failure|cancelled|skipped>   opens, comments on or closes the perf-run-failed issue from the job's result
//
// `plan` needs `gh` and GH_TOKEN (the runner has both) and GITHUB_REPOSITORY.

import { execFileSync } from "node:child_process";
import { appendFileSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { mergesOf, pickBuilds, planMarkdown } from "./lib/nightlyplan.mjs";
import { flaggedMeasures, reportIssue, reportRunFailure } from "./lib/perfissue.mjs";

const WORKFLOW = "ci-nightly-artifacts.yml";
const ARTIFACT = "windows-portable";
const LOOKBACK = 12;

const gh = (args) => JSON.parse(execFileSync(process.env.GH_BIN || "gh", args, { encoding: "utf8", maxBuffer: 64 * 1024 * 1024, shell: Boolean(process.env.GH_BIN) }));

function flag(argv, name) {
    const i = argv.indexOf(name);
    return i >= 0 ? argv[i + 1] : null;
}

/** The last runs of the nightly artifacts workflow, each with whether its windows portable can still be downloaded. */
function fetchRuns(repo) {
    const list = gh(["api", "--method", "GET", `repos/${repo}/actions/workflows/${WORKFLOW}/runs`, "-F", `per_page=${LOOKBACK}`, "-F", "branch=main", "-F", "exclude_pull_requests=true"]);
    // Neither the run's status nor its conclusion is looked at: a red macOS leg, or a rerun of it in progress, leaves the
    // Windows portable as good as it was (2026-10-19: the run was in progress again after a macOS DNS flake).
    return list.workflow_runs
        .map((r) => {
            const arts = gh(["api", "--method", "GET", `repos/${repo}/actions/runs/${r.id}/artifacts`, "-F", "per_page=100"]);
            const portable = arts.artifacts.some((a) => a.name === ARTIFACT && !a.expired);
            return { id: r.id, sha: r.head_sha, createdAt: r.created_at, portable };
        });
}

function plan(argv) {
    const repo = process.env.GITHUB_REPOSITORY;
    if (!repo) throw new Error("GITHUB_REPOSITORY is not set");
    const out = flag(argv, "--out") ?? "plan.json";
    const picked = pickBuilds(fetchRuns(repo), { bRunId: flag(argv, "--b-run") });
    if (!picked.b) throw new Error(picked.reason);
    const merges = picked.a ? mergesOf(gh(["api", "--method", "GET", `repos/${repo}/compare/${picked.a.sha}...${picked.b.sha}`])) : null;
    writeFileSync(out, JSON.stringify({ ...picked, merges }, null, 1));
    const lines = [`b-run=${picked.b.id}`, `b-sha=${picked.b.sha}`, `a-run=${picked.a?.id ?? ""}`, `a-sha=${picked.a?.sha ?? ""}`];
    console.log(lines.join("\n"));
    if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, lines.join("\n") + "\n");
}

/** Whether the first matrix flagged anything (so a confirmation pair is run); sets the `flagged` step output. */
function flagged(argv) {
    let n = 0;
    try {
        n = flaggedMeasures(JSON.parse(readFileSync(flag(argv, "--compare"), "utf8"))).length;
    } catch {
        // no comparison was written (B alone, or the lab did not finish): nothing to confirm
    }
    const line = `flagged=${n > 0}`;
    console.log(`${line} (${n} measure(s) flagged by the first matrix)`);
    if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, line + "\n");
}

/** Opens, comments on or closes the perf-regression issue from the night's result (lib/perfissue.mjs). */
function issue(argv) {
    const read = (name) => {
        const path = flag(argv, name);
        try {
            return path ? JSON.parse(readFileSync(path, "utf8")) : null;
        } catch {
            return null; // the lab did not get as far as writing it: the state is unknown and nothing is done
        }
    };
    const repo = process.env.GITHUB_REPOSITORY;
    const runUrl = `${process.env.GITHUB_SERVER_URL ?? "https://github.com"}/${repo}/actions/runs/${process.env.GITHUB_RUN_ID}`;
    const run = (args) => execFileSync(process.env.GH_BIN || "gh", args, { encoding: "utf8", maxBuffer: 64 * 1024 * 1024, shell: Boolean(process.env.GH_BIN) }).trim();
    reportIssue({ env: { REPO: repo, RUN_URL: runUrl, DRY_RUN: process.env.DRY_RUN }, plan: read("--plan"), initial: read("--initial"), final: read("--final"), gh: run });
}

/** The performance job's own result (the workflow's `needs.compare.result`) as the perf-run-failed issue (lib/perfissue.mjs). */
function failure(argv) {
    const repo = process.env.GITHUB_REPOSITORY;
    const runUrl = `${process.env.GITHUB_SERVER_URL ?? "https://github.com"}/${repo}/actions/runs/${process.env.GITHUB_RUN_ID}`;
    const run = (args) => execFileSync(process.env.GH_BIN || "gh", args, { encoding: "utf8", maxBuffer: 64 * 1024 * 1024, shell: Boolean(process.env.GH_BIN) }).trim();
    reportRunFailure({ env: { REPO: repo, RUN_URL: runUrl, RUN_ID: process.env.GITHUB_RUN_ID, DRY_RUN: process.env.DRY_RUN }, result: flag(argv, "--result"), gh: run });
}

function report(argv) {
    const plan = JSON.parse(readFileSync(flag(argv, "--plan") ?? "plan.json", "utf8"));
    const parts = [planMarkdown(plan)];
    for (const [name, title] of [["--compare", "Comparison"], ["--spread", "Spread over the runs of B"]]) {
        const path = flag(argv, name);
        if (path) parts.push(`### ${title}`, readFileSync(path, "utf8").trim());
    }
    console.log(parts.join("\n\n"));
}

function main() {
    const [cmd, ...rest] = process.argv.slice(2);
    if (cmd === "plan") plan(rest);
    else if (cmd === "report") report(rest);
    else if (cmd === "flagged") flagged(rest);
    else if (cmd === "issue") issue(rest);
    else if (cmd === "failure") failure(rest);
    else {
        console.error("usage: nightly.mjs plan [--b-run <id>] [--out plan.json] | report --plan plan.json [--compare compare.md] [--spread spread.md] | flagged --compare compare.json | issue --plan plan.json --initial compare.json --final final.json | failure --result <job result>");
        process.exit(2);
    }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) {
    try {
        main();
    } catch (e) {
        console.error(e.message);
        process.exit(1);
    }
}
