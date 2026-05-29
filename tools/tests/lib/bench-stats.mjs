// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// bench-stats.mjs — shared statistics + baseline-delta layer for the input
// latency benches (bench-agent-keystroke, bench-term-echo).
//
// Why this exists (input-first Phase 0.1, discussion #1161):
//   The single-run absolute-threshold design (e.g. "fail if P95 > 50 ms") is
//   flaky on shared/variable CI hardware — sub-50 ms timing has high variance,
//   so an absolute gate either false-positives constantly or gets disabled.
//   The fix the reviewers converged on:
//     1. Median-of-N runs with an explicit variance budget.
//     2. Regression = DELTA vs a committed baseline (e.g. +20%), not an
//        absolute number the runner can't reproduce.
//     3. Reporting-mode first (never blocks); promote to gate only once
//        variance is characterized on a pinned device.
//
// These are PURE functions (no I/O except loadBaseline/saveBaseline), so the
// statistical core is unit-tested without needing a running app. See
// bench-stats.test.mjs.

import { readFileSync, writeFileSync, existsSync } from "fs";

// ── Percentiles & single-sample-set summary ─────────────────────────────────

/**
 * Percentile via floored linear index: idx = clamp(floor(N*p/100), 0, N-1).
 * This intentionally matches the convention already used by the benches
 * (bench-agent-keystroke.mjs, bench-term-echo.mjs) so aggregated numbers line
 * up with each bench's own per-run output. It is NOT the strict nearest-rank
 * (ceil) definition. p in [0,100]; returns null for empty input.
 */
export function percentile(arr, p) {
    if (!arr || arr.length === 0) return null;
    const sorted = [...arr].sort((a, b) => a - b);
    const idx = Math.min(sorted.length - 1, Math.max(0, Math.floor((sorted.length * p) / 100)));
    return sorted[idx];
}

export function mean(arr) {
    if (!arr || arr.length === 0) return null;
    return arr.reduce((a, b) => a + b, 0) / arr.length;
}

/** Population standard deviation. null for <2 samples. */
export function stdev(arr) {
    if (!arr || arr.length < 2) return null;
    const m = mean(arr);
    const v = arr.reduce((a, b) => a + (b - m) * (b - m), 0) / arr.length;
    return Math.sqrt(v);
}

/** Summary of one sample set. */
export function summarize(samples) {
    if (!samples || samples.length === 0) {
        return { n: 0, p50: null, p95: null, p99: null, max: null, mean: null, stdev: null };
    }
    return {
        n: samples.length,
        p50: percentile(samples, 50),
        p95: percentile(samples, 95),
        p99: percentile(samples, 99),
        max: Math.max(...samples),
        mean: mean(samples),
        stdev: stdev(samples),
    };
}

// ── Multi-run aggregation ───────────────────────────────────────────────────
//
// A "run" is one full bench invocation producing a sample array. Aggregating
// N runs gives both (a) a pooled summary over all samples and (b) the
// distribution of the per-run P95s — the latter is how we judge whether the
// runner is stable enough to gate on.

/**
 * @param {number[][]} runs - array of per-run sample arrays
 * @returns aggregate with pooled stats, per-run p95 list, median-of-run-p95,
 *          and the run-to-run spread (max-min and coefficient of variation).
 */
export function aggregateRuns(runs) {
    const valid = (runs || []).filter((r) => Array.isArray(r) && r.length > 0);
    if (valid.length === 0) {
        return { runs: 0, pooled: summarize([]), runP95s: [], medianP95: null, p95Spread: null, p95CoV: null };
    }
    const pooled = summarize(valid.flat());
    const runP95s = valid.map((r) => percentile(r, 95));
    const medianP95 = percentile(runP95s, 50);
    const p95Spread = runP95s.length > 1 ? Math.max(...runP95s) - Math.min(...runP95s) : 0;
    const sd = stdev(runP95s);
    const m = mean(runP95s);
    const p95CoV = sd != null && m ? sd / m : null; // coefficient of variation
    return { runs: valid.length, pooled, runP95s, medianP95, p95Spread, p95CoV };
}

// ── Baseline persistence ────────────────────────────────────────────────────

export function loadBaseline(path) {
    if (!path || !existsSync(path)) return null;
    try {
        return JSON.parse(readFileSync(path, "utf8"));
    } catch {
        return null;
    }
}

export function saveBaseline(path, baseline) {
    writeFileSync(path, JSON.stringify(baseline, null, 2));
}

// ── Baseline comparison / verdict ───────────────────────────────────────────

export const VERDICT = Object.freeze({
    NO_BASELINE: "no-baseline",
    PASS: "pass",
    IMPROVE: "improve",
    REGRESS: "regress",
    NOISY: "noisy", // run-to-run variance exceeds budget → verdict untrustworthy
});

/**
 * Compare a current aggregate to a baseline value on one metric (default the
 * median-of-run-P95 — the most stable headline number).
 *
 * @param {object} agg        - output of aggregateRuns()
 * @param {object|null} baseline - { metric, value, ... } or null
 * @param {object} opts
 *   metric        - which agg field to compare (default 'medianP95')
 *   tolerancePct  - allowed regression before flagging (default 20)
 *   maxCoV        - max run-to-run coefficient of variation before 'noisy' (default 0.25)
 * @returns { verdict, current, baseline, deltaPct, tolerancePct, metric, coV }
 */
export function compareToBaseline(agg, baseline, opts = {}) {
    const metric = opts.metric ?? "medianP95";
    const tolerancePct = opts.tolerancePct ?? 20;
    const maxCoV = opts.maxCoV ?? 0.25;
    const current = agg?.[metric] ?? null;

    if (agg && agg.p95CoV != null && agg.p95CoV > maxCoV) {
        return { verdict: VERDICT.NOISY, current, baseline: baseline?.value ?? null, deltaPct: null, tolerancePct, metric, coV: agg.p95CoV };
    }
    // A valid latency baseline is a positive finite number. Reject null / 0 /
    // negative / non-finite (a hand-edited "value": 0 would otherwise divide to
    // Infinity → false REGRESS, or NaN → silent PASS).
    if (!baseline || !Number.isFinite(baseline.value) || baseline.value <= 0) {
        return { verdict: VERDICT.NO_BASELINE, current, baseline: baseline?.value ?? null, deltaPct: null, tolerancePct, metric, coV: agg?.p95CoV ?? null };
    }
    if (current == null) {
        return { verdict: VERDICT.NO_BASELINE, current: null, baseline: baseline.value, deltaPct: null, tolerancePct, metric, coV: agg?.p95CoV ?? null };
    }
    const deltaPct = ((current - baseline.value) / baseline.value) * 100;
    let verdict;
    if (deltaPct > tolerancePct) verdict = VERDICT.REGRESS;
    else if (deltaPct < -tolerancePct) verdict = VERDICT.IMPROVE;
    else verdict = VERDICT.PASS;
    return { verdict, current, baseline: baseline.value, deltaPct, tolerancePct, metric, coV: agg?.p95CoV ?? null };
}

/**
 * Translate a verdict into an exit code, honoring the mode.
 *   mode 'report' — always 0 (never blocks). Use until variance is characterized.
 *   mode 'gate'   — 1 on REGRESS or NOISY; 0 otherwise.
 * NO_BASELINE is 0 in both modes (first run establishes the baseline).
 */
export function verdictExitCode(verdict, mode = "report") {
    if (mode !== "gate") return 0;
    if (verdict === VERDICT.REGRESS || verdict === VERDICT.NOISY) return 1;
    return 0;
}

export function fmtMs(n) {
    return n == null ? "—" : `${n.toFixed(2)} ms`;
}

/** One-line human verdict for console + CI logs. */
export function formatVerdict(cmp, mode) {
    const d = cmp.deltaPct == null ? "" : ` (${cmp.deltaPct >= 0 ? "+" : ""}${cmp.deltaPct.toFixed(1)}% vs baseline)`;
    const cov = cmp.coV == null ? "" : ` [run-to-run CoV ${(cmp.coV * 100).toFixed(1)}%]`;
    const tag = mode === "gate" ? "GATE" : "REPORT";
    return `[${tag}] ${cmp.metric}=${fmtMs(cmp.current)}${d}${cov} → ${cmp.verdict.toUpperCase()}`;
}
