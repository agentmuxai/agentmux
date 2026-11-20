// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The commits a red push run stands for. Pushes to main share one concurrency group, and a run
// waiting while another is in progress is replaced by a newer push, so one run can cover several
// merges: a failure is in one of the commits since the last run that reached a verdict, or in
// their combination. ci-pr.yml's `main status` adds this to the main-red report.
//
// Prints nothing when the run's commit is the one right after the last run's (nothing was
// skipped), or when no earlier run that finished is found; otherwise a short block of Markdown
// listing the commits, newest first.
//
// Env: GH_TOKEN, REPO, SHA (the run's commit), RUN_ID.

import { execFileSync } from "node:child_process";
import { pathToFileURL } from "node:url";

/** The most commits listed; the count above them is still the real one. */
export const LIST_MAX = 20;

/** A run that reached a verdict on its commit. A cancelled or timed-out one did not. */
const VERDICT = new Set(["success", "failure"]);

/**
 * The newest push run before `current` (a run object with `id` and `created_at`) that reached a verdict, out of
 * `runs` (any order), or null.
 */
export function lastVerdictRun(runs, current) {
    return (
        runs
            .filter((r) => r.id !== current.id && VERDICT.has(r.conclusion) && r.created_at < current.created_at)
            .sort((a, b) => (a.created_at < b.created_at ? 1 : -1))[0] ?? null
    );
}

/**
 * The Markdown block for `commits` (`[{sha, subject}]`, newest first) since `previous` (`{head_sha, conclusion}`), or
 * "" when there is at most one commit: the run's own.
 */
export function block(previous, commits, total = commits.length) {
    if (!previous || total <= 1) return "";
    const lines = [
        `The last run that finished tested \`${previous.head_sha.slice(0, 9)}\` (${previous.conclusion}). This run covers the ${total} commits since:`,
        "",
        ...commits.slice(0, LIST_MAX).map((c) => `- \`${c.sha.slice(0, 9)}\` ${c.subject}`),
    ];
    if (total > LIST_MAX) lines.push(`- and ${total - LIST_MAX} older`);
    lines.push("", "The failure is in one of them or in their combination: each passed the merge queue's fast gates, none had this run's full suite.");
    return lines.join("\n");
}

/** Runs gh and returns its stdout. */
function realGh(args) {
    return execFileSync("gh", args, { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 }).trim();
}

/** The whole job. `gh` runs one gh command and returns its stdout. Returns the text to print. */
export function report(env, gh = realGh) {
    const { REPO, SHA, RUN_ID } = env;
    const current = JSON.parse(gh(["api", `repos/${REPO}/actions/runs/${RUN_ID}`]));
    const runs = JSON.parse(gh(["api", `repos/${REPO}/actions/workflows/ci-pr.yml/runs?event=push&branch=main&per_page=50`])).workflow_runs ?? [];
    const previous = lastVerdictRun(runs, current);
    if (!previous || previous.head_sha === SHA) return "";
    const compared = JSON.parse(gh(["api", `repos/${REPO}/compare/${previous.head_sha}...${SHA}`]));
    // The compare answer lists oldest first.
    const commits = (compared.commits ?? [])
        .map((c) => ({ sha: c.sha, subject: String(c.commit?.message ?? "").split("\n")[0] }))
        .reverse();
    return block(previous, commits, compared.total_commits ?? commits.length);
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
    try {
        const text = report(process.env);
        if (text) console.log(text);
    } catch (e) {
        // The red report goes out without it.
        console.error(`ci-main-range: ${String(e.message ?? e).split("\n")[0]}`);
    }
}
