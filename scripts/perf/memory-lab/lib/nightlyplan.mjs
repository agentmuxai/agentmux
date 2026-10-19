// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Which two nightly builds the performance check compares, and what the page says about them. Pure functions; the
// workflow's `nightly.mjs` fetches the runs and the commit list from GitHub and calls these.

/**
 * Picks B (the build under test) and A (what it is compared with) from the nightly artifact runs.
 *
 * A run is usable when it still has its windows-portable artifact (`portable`): the run's own conclusion does not
 * matter, because a red macOS leg fails the run while the Windows portable is fine. B is the newest usable run, or
 * `bRunId` when given. A is the newest usable run older than B whose commit differs from B's: two nights with no merge
 * between them are the same build, and comparing a build with itself says nothing. No A is not an error: B is then
 * measured alone.
 *
 * @param {{id: number, sha: string, createdAt: string, portable: boolean}[]} runs
 * @param {{bRunId?: number|string|null}} [opts]
 */
export function pickBuilds(runs, { bRunId = null } = {}) {
    const usable = runs.filter((r) => r.portable).sort((x, y) => Date.parse(y.createdAt) - Date.parse(x.createdAt));
    const b = bRunId ? usable.find((r) => String(r.id) === String(bRunId)) : usable[0];
    if (!b) return { b: null, a: null, reason: bRunId ? `run ${bRunId} has no windows-portable artifact (expired, or never built)` : "no nightly run has a windows-portable artifact" };
    const older = usable.filter((r) => Date.parse(r.createdAt) < Date.parse(b.createdAt));
    const a = older.find((r) => r.sha !== b.sha) ?? null;
    const reason = a ? null : older.length ? "every older nightly is the same commit as B: nothing to compare with" : "no older nightly still has its portable: B is measured alone";
    return { b, a, reason };
}

const MAX_MERGES = 40;

/** The commits between A and B from GitHub's compare API: `{ total, shown: [{sha, title}], truncated }`. Squash merges are made by the bot; the title names the agent. */
export function mergesOf(compare) {
    const commits = compare?.commits ?? [];
    const shown = commits.slice(-MAX_MERGES).reverse().map((c) => ({
        sha: String(c.sha ?? "").slice(0, 9),
        title: String(c.commit?.message ?? "").split("\n")[0].slice(0, 160),
    }));
    return { total: commits.length, shown, truncated: commits.length > shown.length || compare?.total_commits > commits.length };
}

const day = (iso) => String(iso).slice(0, 10);

/** The markdown above the comparison table: which builds, and what was merged between them. */
export function planMarkdown({ a, b, merges, reason }) {
    const lines = ["## Nightly performance check (report only)", ""];
    const run = (r) => `run ${r.id}, commit \`${r.sha.slice(0, 9)}\`, built ${day(r.createdAt)}`;
    lines.push(`- **B (under test):** ${run(b)}`);
    lines.push(a ? `- **A (compared with):** ${run(a)}` : `- **A:** none (${reason})`);
    if (a && merges) {
        lines.push("", merges.total === 0 ? "Nothing was merged between A and B." : `Merged between A and B (${merges.total}${merges.truncated ? `, newest ${merges.shown.length} shown` : ""}):`);
        for (const m of merges.shown) lines.push(`- \`${m.sha}\` ${m.title}`);
    }
    return lines.join("\n");
}
