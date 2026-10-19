// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The nightly's own red-main alert (docs/specs/PLAN_NIGHTLY_RED_LEGS_ARE_HEARD_2026_10_18.md
// Phase 1). When any job of a nightly run on main fails, open one issue
// labelled nightly-red, or comment on the open one; when a later run is green,
// close it. It reads each job's own conclusion, not the run's: the macOS leg
// is continue-on-error, so the run shows green while that leg is red, which is
// how a macOS test failed for six nights unseen.
//
// Same rules as ci-pr.yml's `main status` job: one issue per red streak, a
// comment per failing run, and a cancelled run says nothing. Unlike pushes to
// main, two nightlies never run at once (the workflow's concurrency group
// queues them), so two runs can't race to create the issue.
//
// Env: GH_TOKEN, REPO, RUN_ID, RUN_URL, REF, SHA. DRY_RUN=1 prints instead of writing.

import { execFileSync } from "node:child_process";
import { pathToFileURL } from "node:url";

export const LABEL = "nightly-red";
export const TITLE = "nightly is red";

/** Jobs that report elsewhere and can't fail the nightly by design. */
const IGNORED = [/^Doc link report/];
/** Conclusions that mean the job didn't pass. A cancelled job is neither red nor green. */
const RED = new Set(["failure", "timed_out", "startup_failure"]);
const MAX_TESTS = 10;

/** The run's finished jobs, minus the ignored ones (and this job, still running). */
export function finishedJobs(jobs) {
    return jobs.filter((j) => j.status === "completed" && !IGNORED.some((re) => re.test(j.name)));
}

/**
 * Whether a job's check-run annotations say it hit its timeout-minutes.
 * GitHub reports such a job as `cancelled`, not `timed_out`, and the run too
 * (the 2026-10-19 macOS test job), so the conclusion alone can't tell it from
 * a run someone cancelled.
 */
export function timedOut(annotations) {
    return annotations.some((a) => /exceeded the maximum execution time/.test(a.message ?? ""));
}

/** "red" if any job failed, "green" if every one passed or was skipped, else "unknown". */
export function verdict(jobs) {
    if (jobs.some((j) => RED.has(j.conclusion))) return "red";
    if (jobs.length > 0 && jobs.every((j) => j.conclusion === "success" || j.conclusion === "skipped")) return "green";
    return "unknown";
}

/** The first failing tests (or compile errors) named in a job's raw log. */
export function failedTests(log, max = MAX_TESTS) {
    const found = [];
    const add = (s) => {
        if (!found.includes(s) && found.length < max) found.push(s);
    };
    for (const raw of log.split(/\r?\n/)) {
        // eslint-disable-next-line no-control-regex -- cargo colours its errors in CI logs
        const line = raw.replace(/\x1b\[[0-9;]*m/g, "").replace(/^\S+Z /, "");
        let m;
        if ((m = /^test (\S+) \.\.\. FAILED/.exec(line))) add(m[1]);
        else if ((m = /^error: could not compile `([^`]+)`/.exec(line))) add(`${m[1]} (did not compile)`);
        else if ((m = /^\s*(?:FAIL|×)\s+(\S+\.test\.\S+.*)$/.exec(line))) add(m[1].trim());
    }
    return found;
}

/** What to do with the issue: { action: "none" | "close" | "comment" | "create", issues? }. */
export function decide(state, openIssues) {
    if (state === "green") return openIssues.length ? { action: "close", issues: openIssues } : { action: "none" };
    if (state !== "red") return { action: "none" };
    return openIssues.length ? { action: "comment", issues: [Math.min(...openIssues)] } : { action: "create" };
}

/** The comment or issue body for a red run. */
export function note({ sha, subject, runUrl, failing }) {
    const lines = [`The nightly failed on main at \`${sha.slice(0, 9)}\` (${subject}): ${runUrl}`, "", "Failing jobs:"];
    for (const j of failing) {
        lines.push(`- ${j.name}${j.conclusion === "failure" ? "" : ` (${j.conclusion.replace("_", " ")})`}`);
        for (const t of j.tests ?? []) lines.push(`  - \`${t}\``);
    }
    return lines.join("\n");
}

function realGh(args) {
    return execFileSync("gh", args, { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 }).trim();
}

/** The whole job; `gh` runs one gh command and returns its stdout. Returns the writes it made (or would make). */
export function report(env, gh = realGh, log = console.log) {
    const { REPO, RUN_ID, RUN_URL, REF, SHA, DRY_RUN } = env;
    const writes = [];
    if (REF !== "refs/heads/main") {
        log(`Run on ${REF}, not main: nothing to report.`);
        return writes;
    }
    const jobs = finishedJobs(JSON.parse(gh(["api", "--paginate", "--slurp", `repos/${REPO}/actions/runs/${RUN_ID}/jobs?per_page=100`])).flatMap((p) => p.jobs)).map((j) => {
        if (j.conclusion !== "cancelled") return j;
        try {
            // Every page: the timeout annotation can come after a page of warnings.
            const annotations = JSON.parse(gh(["api", "--paginate", "--slurp", `repos/${REPO}/check-runs/${j.id}/annotations?per_page=100`])).flat();
            return timedOut(annotations) ? { ...j, conclusion: "timed_out" } : j;
        } catch (e) {
            // Leave it cancelled, which reports nothing, but say why in the log.
            log(`Couldn't read ${j.name}'s annotations, so a timeout can't be told from a cancel: ${String(e.message ?? e).split("\n")[0]}`);
            return j;
        }
    });
    const state = verdict(jobs);
    const open = JSON.parse(gh(["issue", "list", "--repo", REPO, "--label", LABEL, "--state", "open", "--json", "number"])).map((i) => i.number);
    const plan = decide(state, open);
    log(`Nightly is ${state}; open ${LABEL} issues: [${open.join(", ")}]; action: ${plan.action}.`);
    const write = (args) => {
        writes.push(args);
        if (DRY_RUN) log(`[dry run] gh ${args.join(" ")}`);
        else gh(args);
    };

    if (plan.action === "close") {
        for (const n of plan.issues) write(["issue", "close", String(n), "--repo", REPO, "--comment", `Green again at \`${SHA.slice(0, 9)}\`: ${RUN_URL}`]);
    }
    if (plan.action !== "comment" && plan.action !== "create") return writes;

    const failing = jobs
        .filter((j) => RED.has(j.conclusion))
        .map((j) => {
            let tests = [];
            try {
                tests = failedTests(gh(["api", `repos/${REPO}/actions/jobs/${j.id}/logs`]));
            } catch {
                // The log can be missing for a job that never started; the job's name is still reported.
            }
            return { ...j, tests };
        });
    const subject = gh(["api", `repos/${REPO}/commits/${SHA}`, "--jq", '.commit.message | split("\\n")[0]']);
    const body = note({ sha: SHA, subject, runUrl: RUN_URL, failing });
    if (plan.action === "comment") {
        write(["issue", "comment", String(plan.issues[0]), "--repo", REPO, "--body", body]);
        return writes;
    }
    write(["label", "create", LABEL, "--repo", REPO, "--color", "B60205", "--description", "The nightly build or tests are failing", "--force"]);
    const footer = "This issue is opened and closed by CI (ci-nightly-build.yml, nightly status): it stays open while nightlies on main fail, with a comment for each, and closes when one is green.";
    write(["issue", "create", "--repo", REPO, "--title", TITLE, "--label", LABEL, "--body", `${body}\n\n${footer}`]);
    return writes;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) report(process.env);
