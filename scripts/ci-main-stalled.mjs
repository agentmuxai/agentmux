// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A stalled job on main is reported even after main has moved on. ci-pr.yml's
// `main status` job tells a person about a push that failed CI only while that
// push is still main's head, because a code failure is repeated, and fixed or
// not, by the newer push's own run. A job that was cancelled or timed out
// (the Linux test leg running out of memory, 2026-11-16: runs 43946172051 and
// 43954169035) says nothing about the commit, and the newer run neither
// repeats nor fixes it, so nobody was told.
//
// When `CI required` failed and every job that did not pass was cancelled or
// timed out, this reports it to one standing issue labelled ci-stalled
// (created once, reopened if closed, a comment per occurrence, the first too) wherever main's
// head is, and tells the workflow to leave the main-red issue alone. A job
// that failed a step, alone or beside a stalled one, is a real failure and
// stays with `main status`.
//
// Env: GH_TOKEN, REPO, RUN_ID, RUN_URL, SHA, RESULT (CI required's result),
// GITHUB_OUTPUT (the step output `kind`: stalled, real or none).
// DRY_RUN=1 prints the writes instead of making them.

import { appendFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { pathToFileURL } from "node:url";

export const LABEL = "ci-stalled";
export const TITLE = "CI stalled on main";

/** Jobs of the run that are not the CI under test: the gate that reports on them, and this job. */
const OWN_JOBS = new Set(["CI required", "main status"]);
/** A job that did not run to a verdict. */
const STALLED = new Set(["cancelled", "timed_out"]);
/** A job that failed. */
const FAILED = new Set(["failure", "startup_failure"]);

/**
 * Whether a job's check-run annotations say it hit its timeout-minutes. GitHub reports such a
 * job as `cancelled` (the same as one someone cancelled), so the conclusion alone can't say.
 */
export function timedOut(annotations) {
    return annotations.some((a) => /exceeded the maximum execution time/.test(a.message ?? ""));
}

/**
 * "stalled" when something did not pass and every such job was cancelled or timed out,
 * "real" when any job failed (alone or mixed with a stalled one), "none" when nothing did not pass.
 */
export function kindOf(jobs) {
    const finished = jobs.filter((j) => j.status === "completed" && !OWN_JOBS.has(j.name));
    if (finished.some((j) => FAILED.has(j.conclusion))) return "real";
    return finished.some((j) => STALLED.has(j.conclusion)) ? "stalled" : "none";
}

/** The step a job was in when it stopped: the one cancelled, else the first not finished. */
export function stoppedStep(steps = []) {
    const s = steps.find((x) => x.conclusion === "cancelled") ?? steps.find((x) => x.status !== "completed");
    return s?.name ?? null;
}

/** What to do with the standing issue (`issues` are the labelled ones, any state). */
export function decide(issues) {
    if (issues.length === 0) return { action: "create" };
    const keep = issues.reduce((a, b) => (b.number < a.number ? b : a));
    return { action: keep.state === "CLOSED" || keep.state === "closed" ? "reopen" : "comment", issue: keep.number };
}

/** One occurrence's comment: the run, each stalled job with the step it was in, and where its trace is. */
export function note({ sha, subject, runUrl, stalled, traces }) {
    const lines = [`CI stalled on main at \`${sha.slice(0, 9)}\` (${subject}): ${runUrl}`, "", "Stalled jobs:"];
    for (const j of stalled) {
        const how = j.timedOut ? "timed out" : "cancelled";
        lines.push(`- ${j.name} (${how}${j.step ? `, in the step "${j.step}"` : ""})`);
        for (const w of j.warnings ?? []) lines.push(`  - ${w}`);
    }
    lines.push("", traces.length ? `Trace artifact: ${traces.map((t) => `[${t.name}](${t.url})`).join(", ")}` : "Trace artifact: none was kept (a cancelled job's failure-only steps do not run).");
    lines.push("", "Main is not red for this: no code failed. Nothing to revert; the cause is the runner or the test leg's memory.");
    return lines.join("\n");
}

/** Runs gh, trying again after a server error (GitHub's API answers 502 now and then). */
function realGh(args, tries = 3) {
    for (let i = 1; ; i++) {
        try {
            return execFileSync("gh", args, { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 }).trim();
        } catch (e) {
            if (i >= tries) throw e;
            Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 2000 * i);
        }
    }
}

const pages = (json) => JSON.parse(json).flat();

/** The warnings a job's annotations carry about a stall (the nextest step's own "stalled" notice). */
function stallWarnings(annotations) {
    return annotations
        .filter((a) => /stalled/i.test(a.message ?? "") && a.annotation_level === "warning")
        .map((a) => String(a.message).split("\n")[0].slice(0, 300));
}

/** The whole job. `gh` runs one gh command and returns its stdout. Returns the writes it made (or would make). */
export function report(env, gh = realGh, log = console.log) {
    const { REPO, RUN_ID, RUN_URL, SHA, RESULT, DRY_RUN, GITHUB_OUTPUT } = env;
    const writes = [];
    const out = (kind) => {
        log(`kind=${kind}`);
        if (GITHUB_OUTPUT) appendFileSync(GITHUB_OUTPUT, `kind=${kind}\n`);
    };
    if (RESULT !== "failure") {
        out("none");
        return writes;
    }
    const jobs = JSON.parse(gh(["api", "--paginate", "--slurp", `repos/${REPO}/actions/runs/${RUN_ID}/jobs?per_page=100`])).flatMap((p) => p.jobs);
    const kind = kindOf(jobs);
    if (kind !== "stalled") {
        out(kind);
        return writes;
    }

    const stalled = jobs
        .filter((j) => j.status === "completed" && !OWN_JOBS.has(j.name) && STALLED.has(j.conclusion))
        .map((j) => {
            let annotations = [];
            try {
                annotations = pages(gh(["api", "--paginate", "--slurp", `repos/${REPO}/check-runs/${j.id}/annotations?per_page=100`]));
            } catch (e) {
                log(`Couldn't read ${j.name}'s annotations: ${String(e.message ?? e).split("\n")[0]}`);
            }
            return { name: j.name, timedOut: j.conclusion === "timed_out" || timedOut(annotations), step: stoppedStep(j.steps), warnings: stallWarnings(annotations) };
        });
    let traces = [];
    try {
        const arts = JSON.parse(gh(["api", `repos/${REPO}/actions/runs/${RUN_ID}/artifacts?per_page=100`])).artifacts ?? [];
        traces = arts.filter((a) => /trace/i.test(a.name) && !a.expired).map((a) => ({ name: a.name, url: `${RUN_URL}/artifacts/${a.id}` }));
    } catch (e) {
        log(`Couldn't list the run's artifacts: ${String(e.message ?? e).split("\n")[0]}`);
    }
    const subject = gh(["api", `repos/${REPO}/commits/${SHA}`, "--jq", '.commit.message | split("\\n")[0]']);
    const body = note({ sha: SHA, subject, runUrl: RUN_URL, stalled, traces });

    const write = (args) => {
        writes.push(args);
        if (DRY_RUN) {
            log(`[dry run] gh ${args.join(" ")}`);
            return "";
        }
        return gh(args);
    };
    const labelled = () => JSON.parse(gh(["issue", "list", "--repo", REPO, "--label", LABEL, "--state", "all", "--limit", "100", "--json", "number,state"]));
    const plan = decide(labelled());
    log(`Action: ${plan.action}${plan.issue ? ` #${plan.issue}` : ""}.`);
    if (plan.action === "comment") {
        write(["issue", "comment", String(plan.issue), "--repo", REPO, "--body", body]);
    } else if (plan.action === "reopen") {
        write(["issue", "reopen", String(plan.issue), "--repo", REPO, "--comment", body]);
    } else {
        write(["label", "create", LABEL, "--repo", REPO, "--color", "FBCA04", "--description", "A CI job on main was cancelled or timed out; no code failed", "--force"]);
        // Every occurrence is a comment, the first too, so the issue's own text never holds one.
        const description = "A CI job on a push to main was cancelled or timed out with nothing failing. CI (ci-pr.yml, main status) opens this issue once, comments on it for each occurrence, and reopens it if it is closed. It does not mean main is red.";
        const created = write(["issue", "create", "--repo", REPO, "--title", TITLE, "--label", LABEL, "--body", description]);
        const mine = Number(/(\d+)\s*$/.exec(created)?.[1]);
        // Another run may have created one at the same moment: keep the oldest, and close this one's as its duplicate.
        const keep = DRY_RUN || !mine ? mine : Math.min(mine, ...labelled().map((i) => i.number));
        write(["issue", "comment", String(keep || "NEW"), "--repo", REPO, "--body", body]);
        if (keep && mine && keep < mine) {
            try {
                write(["issue", "close", String(mine), "--repo", REPO, "--reason", "not planned", "--comment", `Duplicate of #${keep}, opened by a run at the same moment.`]);
            } catch (e) {
                // The occurrence is already reported; a duplicate left open is only untidy.
                log(`Couldn't close the duplicate #${mine}: ${String(e.message ?? e).split("\n")[0]}`);
            }
        }
    }
    // Only now: a report that failed above leaves `kind` unset, so main status still reports the failure as main-red.
    out("stalled");
    return writes;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) report(process.env);
