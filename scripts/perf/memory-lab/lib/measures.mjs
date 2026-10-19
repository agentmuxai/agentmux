// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The measures of a lab run that the nightly performance check watches, under the names the history line uses
// (SPEC_NIGHTLY_PERF_REGRESSION: perf/history.jsonl; the nightly email reads it). Names are never changed, only added.

import { median } from "./summary.mjs";

/** One run's measures; a measure the run has no figure for is null. */
export function measuresOf(result) {
    const g = result.steady?.groups ?? {};
    const cpu = result.cpu?.seconds ?? {};
    return {
        app_mb: result.steady?.app?.privateMB ?? null,
        renderer_mb: g.renderer?.privateMB ?? null,
        srv_mb: g.srv?.privateMB ?? null,
        app_cpu_s: cpu.app ?? null,
        srv_cpu_s: cpu.srv ?? null,
        renderer_cpu_s: cpu.renderer ?? null,
        gpu_cpu_s: cpu.gpu ?? null,
        round_ms: result.wall?.medianMs ?? null,
        srv_written_mb: result.io?.writtenMB?.srv ?? null,
        store_lock_max_ms: result.storeLock?.longestHeldMs ?? null,
        slow_page_commands: result.slowEvals?.total ?? null,
    };
}

/** The lowest, median and highest of a measure over runs, and how wide that is as a percent of the median; null when no run has it. */
export function spreadOf(values) {
    const v = values.filter((x) => x != null && Number.isFinite(x));
    if (!v.length) return null;
    const min = Math.min(...v);
    const max = Math.max(...v);
    const med = median(v);
    return { n: v.length, min, median: med, max, widthPct: med ? Math.round(((max - min) / med) * 1000) / 10 : null };
}

/** Every measure's spread over several runs of one build. */
export function spreadTable(results) {
    const per = results.map(measuresOf);
    const out = {};
    for (const key of Object.keys(per[0] ?? measuresOf({}))) out[key] = spreadOf(per.map((m) => m[key]));
    return out;
}

/** A markdown table of `spreadTable`, with the wall time of each run beside it when given (seconds). */
export function spreadMarkdown(results, wallSeconds = []) {
    const table = spreadTable(results);
    const f = (x) => (x == null ? "n/a" : String(Math.round(x * 10) / 10));
    const lines = ["| measure | runs | lowest | median | highest | width |", "|---|---:|---:|---:|---:|---:|"];
    for (const [key, s] of Object.entries(table)) {
        lines.push(s ? `| ${key} | ${s.n} | ${f(s.min)} | ${f(s.median)} | ${f(s.max)} | ${s.widthPct == null ? "n/a" : `${s.widthPct}%`} |` : `| ${key} | 0 | n/a | n/a | n/a | n/a |`);
    }
    if (wallSeconds.length) {
        const w = spreadOf(wallSeconds);
        lines.push("", `Wall time per run (the lab step, including starting the instance): ${wallSeconds.map((s) => `${Math.round(s)} s`).join(", ")}; median ${Math.round(w.median)} s.`);
    }
    return lines.join("\n");
}

const ROWS = { app_mb: ["app", "memory"], renderer_mb: ["renderer", "memory"], srv_mb: ["srv", "memory"], app_cpu_s: ["app", "cpu"], srv_cpu_s: ["srv", "cpu"], renderer_cpu_s: ["renderer", "cpu"], gpu_cpu_s: ["gpu", "cpu"] };

/**
 * A `compareRuns` result as the history line's `measures` (SPEC_NIGHTLY_PERF_REGRESSION): each measure with `a` (the base's median),
 * `b` (the head's), `change_pct` and `flag`, under the names `measuresOf` uses; and the names that flagged. Names are never renamed, only added.
 */
export function measuresComparison(cmp) {
    const cell = (m) => (m ? { a: m.base.median ?? null, b: m.head.median ?? null, change_pct: m.pct ?? null, flag: Boolean(m.flagged) } : null);
    const measures = {};
    for (const [name, [group, kind]] of Object.entries(ROWS)) {
        const row = cmp.rows.find((r) => r.group === group);
        measures[name] = cell(row?.[kind]);
    }
    measures.round_ms = cell(cmp.wall);
    measures.srv_written_mb = cell(cmp.written);
    measures.store_lock_max_ms = cell(cmp.lock);
    measures.slow_page_commands = cell(cmp.slowCommands);
    const flags = Object.entries(measures).filter(([, v]) => v?.flag).map(([k]) => k);
    return { measures, flags };
}
