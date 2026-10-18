// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What the memory lab reports, from the per-process numbers: a run's processes
// grouped the way Tower and the 500 MB budget count them, and two runs
// compared. Pure functions, so the rules can be tested without an app.
//
// A sample is one `memtree` sample (crates/procstats/examples/memtree.rs
// --json): every process under the app, with its role (`--type=` of a CEF
// subprocess, "main" otherwise), whether it is the app's own (`app`) or a
// pane's child (an agent CLI, a shell), and its private working set and
// private commit in bytes.

export const MB = 1024 * 1024;

/** The groups, in the order a report lists them. */
export const GROUPS = ["renderer", "gpu", "srv", "host", "utility", "other"];

/** Which group a process of the app's own belongs to. */
export function groupOf(proc) {
    if (proc.role === "renderer") return "renderer";
    if (proc.role === "gpu-process") return "gpu";
    if (proc.role === "utility") return "utility";
    if (proc.role !== "main") return "other"; // crashpad, zygote, ...
    const name = String(proc.name ?? "").toLowerCase();
    if (name.startsWith("agentmux-srv")) return "srv";
    return "host"; // the launcher and the CEF host
}

const empty = () => ({ procs: 0, private: 0, commit: 0 });

/**
 * One sample's processes by group. `app` is the sum of the app's own
 * processes, the number the 500 MB budget is about; `panes` is what the
 * agents' own children (the CLIs, shells) use, which the budget doesn't count.
 * @returns {{ groups: Record<string, {procs:number, private:number, commit:number}>, app: {procs:number, private:number, commit:number}, panes: {procs:number, private:number, commit:number} }}
 */
export function groupSample(sample) {
    const groups = Object.fromEntries(GROUPS.map((g) => [g, empty()]));
    const app = empty();
    const panes = empty();
    for (const p of sample.procs ?? []) {
        const totals = p.app ? [groups[groupOf(p)], app] : [panes];
        for (const t of totals) {
            t.procs += 1;
            t.private += p.private ?? 0;
            t.commit += p.commit ?? 0;
        }
    }
    return { groups, app, panes };
}

export function median(values) {
    const v = values.filter((x) => Number.isFinite(x)).sort((a, b) => a - b);
    if (v.length === 0) return null;
    const mid = v.length >> 1;
    return v.length % 2 ? v[mid] : (v[mid - 1] + v[mid]) / 2;
}

/**
 * The steady value of several samples: the median of each number, so one
 * sample taken mid-garbage-collection doesn't decide it.
 */
export function steady(samples) {
    const grouped = samples.map(groupSample);
    const pick = (get) => median(grouped.map(get));
    const sum = (get) => ({ procs: pick((s) => get(s).procs), private: pick((s) => get(s).private), commit: pick((s) => get(s).commit) });
    const out = { samples: samples.length, groups: {}, app: sum((s) => s.app), panes: sum((s) => s.panes) };
    for (const g of GROUPS) out.groups[g] = sum((s) => s.groups[g]);
    return out;
}

/** The agents whose transcript shows fewer than `min` tool rows: they didn't play a turn. */
export function agentsWithoutTurns(rows, min = 6) {
    return rows.filter((r) => !(r.tools >= min)).map((r) => r.name);
}

/**
 * Why a run's transcripts grew too little for the turns it should have played,
 * or null. `before` and `after` are the transcript store's size (the database
 * and its write-ahead log, in bytes); each played turn adds about what the
 * scripted turn weighs in the conversation, so a store that grew by less than
 * `factor` of that (0.8 by default) means the agents didn't play.
 */
export function transcriptGrowthProblem({ before, after, agents, rounds, bytesPerTurn, factor = 0.8 }) {
    const expected = agents * rounds * bytesPerTurn * factor;
    const grew = after - before;
    if (grew >= expected) return null;
    return `the transcript store grew by ${(grew / MB).toFixed(1)} MB; ${agents} agents playing ${rounds} turns should add at least ${(expected / MB).toFixed(1)} MB`;
}

const mb = (bytes) => (bytes == null ? null : Math.round((bytes / MB) * 10) / 10);

/** A steady summary in megabytes, rounded, for the report file. */
export function inMegabytes(summary) {
    const conv = (x) => ({ procs: x.procs, privateMB: mb(x.private), commitMB: mb(x.commit) });
    return {
        samples: summary.samples,
        groups: Object.fromEntries(Object.entries(summary.groups).map(([g, x]) => [g, conv(x)])),
        app: conv(summary.app),
        panes: conv(summary.panes),
    };
}

/**
 * `head` against `base`, per group and for the app: the change in private
 * working set, in MB and percent, largest increase first. Both are
 * `inMegabytes` summaries.
 */
export function compare(base, head) {
    const row = (name, b, h) => {
        const delta = Math.round(((h.privateMB ?? 0) - (b.privateMB ?? 0)) * 10) / 10;
        const pct = b.privateMB ? Math.round((delta / b.privateMB) * 1000) / 10 : null;
        return { name, baseMB: b.privateMB, headMB: h.privateMB, deltaMB: delta, pct, baseCommitMB: b.commitMB, headCommitMB: h.commitMB };
    };
    const rows = GROUPS.map((g) => row(g, base.groups[g], head.groups[g])).filter((r) => r.baseMB || r.headMB);
    rows.sort((a, b) => b.deltaMB - a.deltaMB);
    return { app: row("app", base.app, head.app), groups: rows };
}

/** A markdown table of one summary. */
export function table(summary, title = "") {
    const lines = [];
    if (title) lines.push(`### ${title}`, "");
    lines.push("| group | processes | private MB | commit MB |", "|---|---:|---:|---:|");
    for (const g of GROUPS) {
        const x = summary.groups[g];
        if (x.procs) lines.push(`| ${g} | ${x.procs} | ${x.privateMB} | ${x.commitMB} |`);
    }
    lines.push(`| **app total** | ${summary.app.procs} | **${summary.app.privateMB}** | ${summary.app.commitMB} |`);
    lines.push(`| panes' children (not counted) | ${summary.panes.procs} | ${summary.panes.privateMB} | ${summary.panes.commitMB} |`);
    return lines.join("\n");
}

/** A markdown table of `compare`. */
export function comparisonTable(cmp, baseLabel = "base", headLabel = "head") {
    const sign = (n) => (n > 0 ? `+${n}` : String(n));
    const lines = [`| group | ${baseLabel} MB | ${headLabel} MB | change MB | change % |`, "|---|---:|---:|---:|---:|"];
    for (const r of cmp.groups) lines.push(`| ${r.name} | ${r.baseMB} | ${r.headMB} | ${sign(r.deltaMB)} | ${r.pct == null ? "-" : sign(r.pct) + "%"} |`);
    const a = cmp.app;
    lines.push(`| **app total** | ${a.baseMB} | ${a.headMB} | **${sign(a.deltaMB)}** | ${a.pct == null ? "-" : "**" + sign(a.pct) + "%**"} |`);
    return lines.join("\n");
}
