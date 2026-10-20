// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The `perf-regression` issue (SPEC_NIGHTLY_PERF_REGRESSION §4.5). Pure functions; `nightly.mjs issue` makes the gh calls, and the
// open/comment/close rule is `decide` of scripts/ci-nightly-status.mjs (one issue per streak, a comment for each later flagged
// night, closed by a clean one).

import { decide, timedOut } from "../../../ci-nightly-status.mjs";

export const PERF_LABEL = "perf-regression";
export const PERF_TITLE = "nightly performance regression";
export const DRIFT_TITLE = "nightly performance drift";

/** The measures in a compare.json (`measuresComparison`) that are flagged, in the order they appear. */
export function flaggedMeasures(compareJson) {
    return Object.entries(compareJson?.measures ?? {})
        .filter(([, m]) => m?.flag)
        .map(([name, m]) => ({ name, a: m.a, b: m.b, changePct: m.change_pct }));
}

/**
 * What the night says, for the issue rule:
 * - "red": the comparison was made and a flag holds on all the runs together (the confirmation pair included);
 * - "green": the comparison was made and nothing is flagged;
 * - "unknown": there was nothing to compare, or the lab did not finish, or the first matrix flagged something and the confirmation
 *   pair added no runs (so the flag is not confirmed): no issue is opened, commented on or closed.
 *
 * `initial`: the first matrix's compare.json; `final`: the compare.json over every run (the same file when no confirmation was needed).
 */
export function perfState({ hasA, initial, final }) {
    if (!hasA || !final || !initial) return "unknown";
    if (flaggedMeasures(initial).length && !confirmationRan(initial, final)) return "unknown";
    return flaggedMeasures(final).length ? "red" : "green";
}

/** Whether a confirmation pair finished: the comparison over every run has more runs than the first matrix's on both sides (a lone extra B run is not a pair). */
const confirmationRan = (initial, final) => (final?.head?.length ?? 0) > (initial?.head?.length ?? 0) && (final?.base?.length ?? 0) > (initial?.base?.length ?? 0);

const fmt = (x) => (x == null ? "n/a" : String(Math.round(x * 10) / 10));
const sign = (n) => (n == null ? "n/a" : n > 0 ? `+${n}%` : `${n}%`);

/** The issue's body, or a comment's: what is flagged, between which builds, what was merged between them, and where the numbers are. */
export function issueBody({ plan, initial, final, runUrl, kind = "regression", drift = [] }) {
    const flagged = flaggedMeasures(final);
    const pair = confirmationRan(initial, final);
    const run = (r) => `\`${r.sha.slice(0, 9)}\` (nightly artifacts run ${r.id}, ${String(r.createdAt).slice(0, 10)})`;
    const lines = [];
    lines.push(
        kind === "drift"
            ? "The nightly performance check sees a slow drift: no single night flagged, but the last 7 nights' median is more than 15% above the 30 nights before it for the measures below."
            : `The nightly performance check flagged ${flagged.length === 1 ? "a measure" : `${flagged.length} measures`} on all of its runs together${pair ? " (a confirmation pair was run)" : ""}: B is worse than A by more than the margin in \`scripts/perf/memory-lab/budget.json\`, and the two sides' ranges do not overlap.`
    );
    if (kind === "drift") {
        lines.push("", "| measure | the 30 nights before | the last 7 nights | change |", "|---|---:|---:|---:|");
        for (const d of drift) lines.push(`| ${d.name} | ${fmt(d.before)} | ${fmt(d.recent)} | ${sign(d.changePct)} |`);
    } else {
        lines.push("", "| measure | A | B | change |", "|---|---:|---:|---:|");
        for (const f of flagged) lines.push(`| ${f.name} | ${fmt(f.a)} | ${fmt(f.b)} | ${sign(f.changePct)} |`);
    }
    lines.push("", `- **A:** ${plan.a ? run(plan.a) : "none"}`, `- **B:** ${run(plan.b)}`);
    if (plan.merges?.total) {
        lines.push("", `Merged between A and B (${plan.merges.total}${plan.merges.truncated ? `, newest ${plan.merges.shown.length} shown` : ""}):`);
        for (const m of plan.merges.shown) lines.push(`- \`${m.sha}\` ${m.title}`);
    }
    lines.push("", `The run, its job summary (every measure, with ranges) and its artifacts (each run's result.json): ${runUrl}`);
    return lines.join("\n");
}

/** The comment that closes an issue when a night comes back clean. */
export function closeNote({ plan, runUrl }) {
    return `Clean again: nothing is flagged between ${plan.a ? `\`${plan.a.sha.slice(0, 9)}\`` : "the previous build"} and \`${plan.b.sha.slice(0, 9)}\`: ${runUrl}`;
}

export const ISSUE_FOOTER =
    "This issue is opened and closed by CI (perf-regression.yml): it stays open while nights are flagged, with a comment for each, and closes when one is clean. Margins are in scripts/perf/memory-lab/budget.json; the design is SPEC_NIGHTLY_PERF_REGRESSION.";

/**
 * The whole issue step. `gh(args)` runs one gh command and returns its stdout; returns the writes it made (or would make with DRY_RUN).
 * `env`: REPO, RUN_URL, DRY_RUN. `plan`: `nightly.mjs plan`'s plan.json. `initial` / `final`: compare.json of the first matrix and of all runs.
 * `drift`: `{ current, onset }` from `driftOnset` (lib/perfhistory.mjs): a night that flags nothing but sits in a drift does not close the
 * issue, and comments on it (or opens one, titled as a drift) on the night the drift starts, not on every night it lasts.
 */
export function reportIssue({ env, plan, initial, final, drift = { current: [], onset: [] }, gh, log = console.log }) {
    const { REPO, RUN_URL, DRY_RUN } = env;
    const writes = [];
    const state = perfState({ hasA: Boolean(plan?.a), initial, final });
    const open = JSON.parse(gh(["issue", "list", "--repo", REPO, "--label", PERF_LABEL, "--state", "open", "--json", "number"])).map((i) => i.number);
    const drifting = state === "green" && drift.current.length > 0;
    const action = drifting ? (drift.onset.length ? (open.length ? { action: "comment", issues: [Math.min(...open)] } : { action: "create" }) : { action: "none" }) : decide(state, open);
    log(`The nightly performance check is ${state}${drifting ? ` with a drift in ${drift.current.map((d) => d.name).join(", ")}` : ""}; open ${PERF_LABEL} issues: [${open.join(", ")}]; action: ${action.action}.`);
    const write = (args) => {
        writes.push(args);
        if (DRY_RUN) log(`[dry run] gh ${args.join(" ")}`);
        else gh(args);
    };
    const kind = drifting ? "drift" : "regression";
    if (action.action === "close") {
        for (const n of action.issues) write(["issue", "close", String(n), "--repo", REPO, "--comment", closeNote({ plan, runUrl: RUN_URL })]);
    } else if (action.action === "comment") {
        write(["issue", "comment", String(action.issues[0]), "--repo", REPO, "--body", issueBody({ plan, initial, final, runUrl: RUN_URL, kind, drift: drift.current })]);
    } else if (action.action === "create") {
        write(["label", "create", PERF_LABEL, "--repo", REPO, "--color", "D93F0B", "--description", "The nightly performance check flagged a regression", "--force"]);
        write(["issue", "create", "--repo", REPO, "--title", drifting ? DRIFT_TITLE : PERF_TITLE, "--label", PERF_LABEL, "--body", `${issueBody({ plan, initial, final, runUrl: RUN_URL, kind, drift: drift.current })}\n\n${ISSUE_FOOTER}`]);
    }
    return writes;
}

export const FAILED_LABEL = "perf-run-failed";
export const FAILED_TITLE = "nightly perf run failed";

/**
 * What the performance job's own result says, for the same one-issue-per-streak rule: "red" when the job failed or timed out,
 * "green" when it succeeded, "unknown" for a cancelled or skipped job (a cancelled run says nothing, as for the nightly).
 * GitHub reports a job that hit its timeout-minutes as `cancelled` too: `timedOutResult` tells the two apart first.
 */
export function runState(result) {
    if (result === "failure") return "red";
    if (result === "success") return "green";
    return "unknown";
}

/**
 * "failure" for a `cancelled` result whose job hit its timeout (the check-run annotations say so, as `ci-nightly-status.mjs` reads them),
 * else the result as it was. `jobs`: the run's jobs; `annotationsOf(job)`: that job's check-run annotations.
 */
export function timedOutResult(result, jobs, annotationsOf) {
    if (result !== "cancelled") return result;
    for (const job of jobs ?? []) {
        if (job.conclusion !== "cancelled") continue;
        try {
            if (timedOut(annotationsOf(job))) return "failure";
        } catch {
            // the annotations could not be read: stay cancelled, which reports nothing
        }
    }
    return result;
}

/** The failing steps of the performance job, from the jobs API's `jobs` (`conclusion` of the job and of each step). */
export function failedSteps(jobs) {
    const names = [];
    for (const job of jobs ?? []) {
        // a timed-out job is `cancelled`, and so are its steps from the one that was running
        if (job.conclusion !== "failure" && job.conclusion !== "timed_out" && !(job.conclusion === "cancelled" && job.timedOut)) continue;
        const steps = (job.steps ?? []).filter((st) => st.conclusion === "failure" || st.conclusion === "timed_out" || (job.timedOut && st.conclusion === "cancelled")).map((st) => st.name);
        names.push(...(steps.length ? steps.map((n) => `${job.name}: ${n}`) : [job.name]));
    }
    return names;
}

/** The body of the issue, or of a comment, for a run that failed. */
export function failedBody({ runUrl, steps }) {
    const lines = [`The nightly performance check did not finish, so tonight's history line and comparison are missing: ${runUrl}`];
    if (steps.length) lines.push("", "Failed:", ...steps.map((n) => `- ${n}`));
    return lines.join("\n");
}

/** Opens, comments on or closes the `perf-run-failed` issue from the job's result; the rule is `decide`. Returns the writes it made (or would with DRY_RUN). */
export function reportRunFailure({ env, result: given, gh, log = console.log }) {
    const { REPO, RUN_URL, DRY_RUN } = env;
    const writes = [];
    let jobs = [];
    try {
        jobs = JSON.parse(gh(["api", "--paginate", "--slurp", `repos/${REPO}/actions/runs/${env.RUN_ID}/jobs?per_page=100`])).flatMap((p) => p.jobs);
    } catch {
        // without the jobs a timeout cannot be told from a cancel, and the steps are not named: the link says the rest
    }
    const annotationsOf = (job) => JSON.parse(gh(["api", "--paginate", "--slurp", `repos/${REPO}/check-runs/${job.id}/annotations?per_page=100`])).flat();
    const result = timedOutResult(given, jobs, annotationsOf);
    if (result !== given) jobs = jobs.map((j) => (j.conclusion === "cancelled" ? { ...j, timedOut: true } : j));
    const state = runState(result);
    const open = JSON.parse(gh(["issue", "list", "--repo", REPO, "--label", FAILED_LABEL, "--state", "open", "--json", "number"])).map((i) => i.number);
    const action = decide(state, open);
    log(`The nightly performance job ${result}${result === given ? "" : ` (${given}: it hit its timeout)`}; open ${FAILED_LABEL} issues: [${open.join(", ")}]; action: ${action.action}.`);
    const write = (args) => {
        writes.push(args);
        if (DRY_RUN) log(`[dry run] gh ${args.join(" ")}`);
        else gh(args);
    };
    if (action.action === "none") return writes;
    if (action.action === "close") {
        for (const n of action.issues) write(["issue", "close", String(n), "--repo", REPO, "--comment", `The nightly performance check finished again: ${RUN_URL}`]);
        return writes;
    }
    const body = failedBody({ runUrl: RUN_URL, steps: failedSteps(jobs) });
    if (action.action === "comment") write(["issue", "comment", String(action.issues[0]), "--repo", REPO, "--body", body]);
    else {
        write(["label", "create", FAILED_LABEL, "--repo", REPO, "--color", "B60205", "--description", "The nightly performance check did not finish", "--force"]);
        write(["issue", "create", "--repo", REPO, "--title", FAILED_TITLE, "--label", FAILED_LABEL, "--body", `${body}\n\nOpened and closed by CI (perf-regression.yml): one issue while nights keep failing, with a comment for each; closed by the first night that finishes.`]);
    }
    return writes;
}
