// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The `perf-regression` issue (SPEC_NIGHTLY_PERF_REGRESSION §4.5). Pure functions; `nightly.mjs issue` makes the gh calls, and the
// open/comment/close rule is `decide` of scripts/ci-nightly-status.mjs (one issue per streak, a comment for each later flagged
// night, closed by a clean one).

import { decide } from "../../../ci-nightly-status.mjs";

export const PERF_LABEL = "perf-regression";
export const PERF_TITLE = "nightly performance regression";

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
 * - "unknown": there was nothing to compare, or the lab did not finish: no issue is opened, commented on or closed.
 *
 * `initial`: the first matrix's compare.json; `final`: the compare.json over every run (the same file when no confirmation was needed).
 */
export function perfState({ hasA, initial, final }) {
    if (!hasA || !final || !initial) return "unknown";
    return flaggedMeasures(final).length ? "red" : "green";
}

const fmt = (x) => (x == null ? "n/a" : String(Math.round(x * 10) / 10));
const sign = (n) => (n == null ? "n/a" : n > 0 ? `+${n}%` : `${n}%`);

/** The issue's body, or a comment's: what is flagged, between which builds, what was merged between them, and where the numbers are. */
export function issueBody({ plan, initial, final, runUrl, kind = "regression" }) {
    const flagged = flaggedMeasures(final);
    const initiallyFlagged = flaggedMeasures(initial).map((f) => f.name);
    const run = (r) => `\`${r.sha.slice(0, 9)}\` (nightly artifacts run ${r.id}, ${String(r.createdAt).slice(0, 10)})`;
    const lines = [];
    lines.push(
        kind === "drift"
            ? "The nightly performance check sees a slow drift: no single night flagged, but the last 7 nights' median is more than 15% above the 30 nights before it for the measures below."
            : `The nightly performance check flagged ${flagged.length === 1 ? "a measure" : `${flagged.length} measures`} on all of its runs together${initiallyFlagged.length ? " (a confirmation pair was run)" : ""}: B is worse than A by more than the margin in \`scripts/perf/memory-lab/budget.json\`, and the two sides' ranges do not overlap.`
    );
    lines.push("", "| measure | A | B | change |", "|---|---:|---:|---:|");
    for (const f of flagged) lines.push(`| ${f.name} | ${fmt(f.a)} | ${fmt(f.b)} | ${sign(f.changePct)} |`);
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
 */
export function reportIssue({ env, plan, initial, final, gh, log = console.log }) {
    const { REPO, RUN_URL, DRY_RUN } = env;
    const writes = [];
    const state = perfState({ hasA: Boolean(plan?.a), initial, final });
    const open = JSON.parse(gh(["issue", "list", "--repo", REPO, "--label", PERF_LABEL, "--state", "open", "--json", "number"])).map((i) => i.number);
    const action = decide(state, open);
    log(`The nightly performance check is ${state}; open ${PERF_LABEL} issues: [${open.join(", ")}]; action: ${action.action}.`);
    const write = (args) => {
        writes.push(args);
        if (DRY_RUN) log(`[dry run] gh ${args.join(" ")}`);
        else gh(args);
    };
    if (action.action === "close") {
        for (const n of action.issues) write(["issue", "close", String(n), "--repo", REPO, "--comment", closeNote({ plan, runUrl: RUN_URL })]);
    } else if (action.action === "comment") {
        write(["issue", "comment", String(action.issues[0]), "--repo", REPO, "--body", issueBody({ plan, initial, final, runUrl: RUN_URL })]);
    } else if (action.action === "create") {
        write(["label", "create", PERF_LABEL, "--repo", REPO, "--color", "D93F0B", "--description", "The nightly performance check flagged a regression", "--force"]);
        write(["issue", "create", "--repo", REPO, "--title", PERF_TITLE, "--label", PERF_LABEL, "--body", `${issueBody({ plan, initial, final, runUrl: RUN_URL })}\n\n${ISSUE_FOOTER}`]);
    }
    return writes;
}
