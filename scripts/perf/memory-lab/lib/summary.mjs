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

/**
 * Which group a process of the app's own belongs to. The role is the `--type=` of a
 * Chromium subprocess or, from the CDP process list, Chromium's own type: Windows says
 * `gpu-process` and `utility`, Linux `GPU`, and names a utility process by its service
 * (`storage.mojom.StorageService`). The zygote and what it forks without a role of
 * their own are Chromium's helper processes, counted with the utility ones.
 */
export function groupOf(proc) {
    const role = String(proc.role ?? "");
    if (role === "renderer") return "renderer";
    if (/^gpu/i.test(role)) return "gpu";
    if (role === "utility" || role.includes(".mojom.") || role === "zygote" || role === "zygote-forked") return "utility";
    if (role !== "main") return "other"; // crashpad, ...
    const name = String(proc.name ?? "").toLowerCase();
    if (name.startsWith("agentmux-srv")) return "srv";
    return "host"; // the launcher and the CEF host
}

const empty = () => ({ procs: 0, private: 0, commit: 0 });

/** `total + value`, or null once any value is missing: a total over processes whose memory couldn't be read isn't a total. */
const plus = (total, value) => (total == null || value == null ? null : total + value);

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
            t.private = plus(t.private, p.private);
            t.commit = plus(t.commit, p.commit);
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
        const known = b.privateMB != null && h.privateMB != null; // a missing number is not zero
        const delta = known ? Math.round((h.privateMB - b.privateMB) * 10) / 10 : null;
        const pct = known && b.privateMB ? Math.round((delta / b.privateMB) * 1000) / 10 : null;
        return { name, baseMB: b.privateMB, headMB: h.privateMB, deltaMB: delta, pct, baseCommitMB: b.commitMB, headCommitMB: h.commitMB };
    };
    const rows = GROUPS.map((g) => row(g, base.groups[g], head.groups[g])).filter((r) => r.baseMB !== 0 || r.headMB !== 0);
    rows.sort((a, b) => (b.deltaMB ?? -Infinity) - (a.deltaMB ?? -Infinity));
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
    const sign = (n) => (n == null ? "n/a" : n > 0 ? `+${n}` : String(n));
    const lines = [`| group | ${baseLabel} MB | ${headLabel} MB | change MB | change % |`, "|---|---:|---:|---:|---:|"];
    for (const r of cmp.groups) lines.push(`| ${r.name} | ${r.baseMB} | ${r.headMB} | ${sign(r.deltaMB)} | ${r.pct == null ? "-" : sign(r.pct) + "%"} |`);
    const a = cmp.app;
    lines.push(`| **app total** | ${a.baseMB} | ${a.headMB} | **${sign(a.deltaMB)}** | ${a.pct == null ? "-" : "**" + sign(a.pct) + "%**"} |`);
    return lines.join("\n");
}

/** The time the agents took to finish each round: the median, the slowest, and how many rounds could not be timed. */
export function wallSummary(roundMs) {
    const timed = roundMs.filter((x) => Number.isFinite(x));
    return {
        roundMs,
        timedRounds: timed.length,
        untimedRounds: roundMs.length - timed.length,
        medianMs: timed.length ? Math.round(median(timed)) : null,
        maxMs: timed.length ? Math.max(...timed) : null,
    };
}

/** A markdown table of a run's CPU seconds per group and its round times. */
export function cpuTable(cpu, wall, io = null) {
    const lines = ["| group | CPU seconds |", "|---|---:|"];
    for (const g of GROUPS) if (cpu.seconds[g]) lines.push(`| ${g} | ${cpu.seconds[g]} |`);
    lines.push(`| **app total** | **${cpu.seconds.app}** |`);
    if (io) {
        const mbOrNA = (v) => (v == null ? "n/a" : `${v} MB`);
        const storage = io.storageWrittenMB ? ` Of that, reached storage: srv ${mbOrNA(io.storageWrittenMB.srv)}, app total ${mbOrNA(io.storageWrittenMB.app)}.` : "";
        lines.push("", `Bytes written during the workload: srv ${mbOrNA(io.writtenMB.srv)}, renderers ${mbOrNA(io.writtenMB.renderer)}, app total ${mbOrNA(io.writtenMB.app)}${io.incompleteGroups?.length ? ` (n/a: a process in ${io.incompleteGroups.filter((g) => g !== "app").join(", ")} gave no counter)` : ""}.${storage}`);
    }
    lines.push("", `Round time (send to all agents idle): median ${wall.medianMs ?? "n/a"} ms, slowest ${wall.maxMs ?? "n/a"} ms, ${wall.untimedRounds} of ${wall.roundMs.length} rounds not timed.`);
    return lines.join("\n");
}

const pctChange = (base, head) => (base == null || head == null || base === 0 ? null : Math.round(((head - base) / base) * 1000) / 10);

/** A page command that took longer than this during a run is itself a responsiveness signal. */
export const SLOW_COMMAND_MS = 10_000;

/**
 * The percent past which a change is flagged (report only: it decides nothing).
 * Per metric, because the noise differs: app and srv CPU are steady, renderer
 * CPU and the round time move more (measured spread of identical runs: about 5%,
 * 3%, 13% and 15%).
 */
export const DEFAULT_MARGINS = { cpuPct: { app: 10, srv: 10, renderer: 10, gpu: 20, default: 20 }, wallPct: 20, writtenPct: 3, memoryPct: null };

/** The CPU margin of a group; a single number means every group. */
const marginFor = (margins, group) => {
    const c = margins.cpuPct;
    return typeof c === "number" ? c : (c[group] ?? c.default);
};

/**
 * Several runs of a build against several of another: per metric the median
 * of each side, the change in percent and, to judge the noise, each side's
 * lowest and highest. A CPU or round-time change is flagged only when it is past
 * its margin AND the two sides' ranges do not overlap (the lowest run of the
 * head is still above the highest of the base).
 * @param {object[]} baseRuns result.json contents
 * @param {object[]} headRuns
 * @param {{ cpuPct: number | Record<string, number>, wallPct: number }} margins
 */
export function compareRuns(baseRuns, headRuns, margins = DEFAULT_MARGINS) {
    const side = (runs, get) => {
        const v = runs.map(get).filter((x) => x != null && Number.isFinite(x));
        return { median: median(v), min: v.length ? Math.min(...v) : null, max: v.length ? Math.max(...v) : null, n: v.length };
    };
    const metric = (name, unit, get, marginPct = null) => {
        const base = side(baseRuns, get);
        const head = side(headRuns, get);
        const pct = pctChange(base.median, head.median);
        const separate = base.min != null && head.min != null && head.min > base.max; // the ranges do not overlap
        return { name, unit, base, head, pct, marginPct, flagged: marginPct != null && pct != null && pct > marginPct && separate };
    };
    const rows = [];
    for (const g of ["app", ...GROUPS]) {
        const mem = (r) => (g === "app" ? r.steady?.app?.privateMB : r.steady?.groups?.[g]?.privateMB);
        const cpu = (r) => r.cpu?.seconds?.[g];
        const everyone = [...baseRuns, ...headRuns];
        if (side(everyone, mem).median === 0 && !side(everyone, cpu).median) continue; // a group that isn't there
        // Memory is noisy on a runner (13 to 30% between identical runs), so it is report-only unless the margins name one (memoryPct).
        rows.push({ group: g, memory: metric(g, "MB", mem, margins.memoryPct ?? null), cpu: metric(g, "s", cpu, marginFor(margins, g)) });
    }
    const wall = metric("round time", "ms", (r) => r.wall?.medianMs, margins.wallPct);
    const slow = [...baseRuns.map((r) => ["base", r]), ...headRuns.map((r) => ["head", r])]
        .filter(([, r]) => r.slowestEvalMs > SLOW_COMMAND_MS)
        .map(([side, r]) => ({ side, label: r.label, ms: r.slowestEvalMs }));
    const lock = metric("store lock", "ms", (r) => r.storeLock?.longestHeldMs);
    const written = metric("srv written", "MB", (r) => r.io?.writtenMB?.srv, margins.writtenPct ?? DEFAULT_MARGINS.writtenPct);
    const storageWritten = metric("srv reached storage", "MB", (r) => r.io?.storageWrittenMB?.srv, margins.writtenPct ?? DEFAULT_MARGINS.writtenPct);
    const slowCommands = metric("slow page commands", "", (r) => r.slowEvals?.total);
    // Opening an agent again (--open-agents): report only until the noise is known.
    const opens = {
        regular: metric("open first screen", "ms", (r) => r.opens?.summary?.regular?.firstScreenMedianMs),
        heavy: metric("open first screen, long conversation", "ms", (r) => r.opens?.summary?.heavy?.firstScreenMedianMs),
        longestTask: metric("longest main-thread task while opening", "ms", (r) => (r.opens ? Math.max(0, ...Object.values(r.opens.summary ?? {}).map((s) => s.longestTaskMs ?? 0)) : null)),
    };
    // What srv wrote while the first agent played its heavy turns (--heavy-turns): report only, the number a write-amplification change moves.
    const heavyWritten = metric("srv written in the heavy turns", "MB", (r) => r.heavyIo?.writtenMB?.srv);
    const heavyStorageWritten = metric("srv reached storage in the heavy turns", "MB", (r) => r.heavyIo?.storageWrittenMB?.srv);
    return { rows, wall, slow, lock, written, storageWritten, slowCommands, opens, heavyWritten, heavyStorageWritten, margins, baseRuns: baseRuns.length, headRuns: headRuns.length };
}

/** A markdown table of `compareRuns`: memory and CPU side by side, then the round time. */
export function runsTable(cmp, baseLabel = "base", headLabel = "head") {
    const f = (x) => (x == null ? "n/a" : Math.round(x * 10) / 10);
    const sign = (n) => (n == null ? "-" : n > 0 ? `+${n}%` : `${n}%`);
    const range = (s) => (s.n > 1 ? ` (${f(s.min)}-${f(s.max)})` : "");
    const lines = [
        `| group | ${baseLabel} MB | ${headLabel} MB | memory | ${baseLabel} CPU s | ${headLabel} CPU s | CPU | |`,
        "|---|---:|---:|---:|---:|---:|---:|---|",
    ];
    for (const r of cmp.rows) {
        const m = r.memory;
        const c = r.cpu;
        lines.push(`| ${r.group === "app" ? "**app total**" : r.group} | ${f(m.base.median)}${range(m.base)} | ${f(m.head.median)}${range(m.head)} | ${sign(m.pct)} | ${f(c.base.median)}${range(c.base)} | ${f(c.head.median)}${range(c.head)} | ${sign(c.pct)} | ${[m.flagged ? `memory over +${m.marginPct}%` : "", c.flagged ? `CPU over +${c.marginPct}%` : ""].filter(Boolean).join("; ")} |`);
    }
    const w = cmp.wall;
    lines.push("", `Round time (send to all agents idle), median: ${baseLabel} ${f(w.base.median)} ms${range(w.base)}, ${headLabel} ${f(w.head.median)} ms${range(w.head)}, ${sign(w.pct)}${w.flagged ? ` (over +${w.marginPct}%)` : ""}.`);
    const wr = cmp.written;
    if (wr && (wr.base.n || wr.head.n)) {
        lines.push("", `Bytes srv wrote during the workload (report only; what the OS counts is in result.io.note): ${baseLabel} ${f(wr.base.median)} MB${range(wr.base)}, ${headLabel} ${f(wr.head.median)} MB${range(wr.head)}${wr.pct == null ? "" : `, ${sign(wr.pct)}`}${wr.flagged ? ` (over +${wr.marginPct}%)` : ""}.`);
    }
    const sw = cmp.storageWritten;
    if (sw && (sw.base.n || sw.head.n)) {
        lines.push("", `Of that, bytes that reached storage (Linux write_bytes, macOS disk bytes; report only): ${baseLabel} ${f(sw.base.median)} MB${range(sw.base)}, ${headLabel} ${f(sw.head.median)} MB${range(sw.head)}${sw.pct == null ? "" : `, ${sign(sw.pct)}`}${sw.flagged ? ` (over +${sw.marginPct}%)` : ""}.`);
    }
    const l = cmp.lock;
    if (l && (l.base.n || l.head.n)) {
        lines.push("", `Longest hold of srv's transcript store connection (report only; every store write waits for it): ${baseLabel} ${f(l.base.median)} ms${range(l.base)}, ${headLabel} ${f(l.head.median)} ms${range(l.head)}${l.pct == null ? "" : `, ${sign(l.pct)}`}. A hold of 0 means none reached the 500 ms srv logs.`);
    }
    for (const h of [cmp.heavyWritten, cmp.heavyStorageWritten]) {
        if (h && (h.base.n || h.head.n)) lines.push("", `${h.name[0].toUpperCase()}${h.name.slice(1)} (report only), median: ${baseLabel} ${f(h.base.median)} MB${range(h.base)}, ${headLabel} ${f(h.head.median)} MB${range(h.head)}${h.pct == null ? "" : `, ${sign(h.pct)}`}.`);
    }
    for (const o of Object.values(cmp.opens ?? {})) {
        if (o.base.n || o.head.n) lines.push("", `${o.name[0].toUpperCase()}${o.name.slice(1)} (report only), median: ${baseLabel} ${f(o.base.median)} ms${range(o.base)}, ${headLabel} ${f(o.head.median)} ms${range(o.head)}${o.pct == null ? "" : `, ${sign(o.pct)}`}.`);
    }
    for (const s of cmp.slow ?? []) lines.push("", `SLOW PAGE COMMAND: ${s.label} (${s.side}) had a page command that took ${Math.round(s.ms / 100) / 10} s (over ${SLOW_COMMAND_MS / 1000} s): the window was unresponsive for that long during a round.`);
    lines.push("", `${cmp.baseRuns} run(s) against ${cmp.headRuns}; medians, with the lowest-highest in brackets when there is more than one. A flag needs the change to pass the margin (${typeof cmp.margins.cpuPct === "number" ? `+${cmp.margins.cpuPct}%` : `+${cmp.margins.cpuPct.app}% app and srv CPU, +${cmp.margins.cpuPct.default}% other groups`} CPU, +${cmp.margins.wallPct}% round time) and the two sides' ranges not to overlap. Flags are report-only; the operator sets the real margins.`);
    return lines.join("\n");
}
