// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for bench-stats.mjs — the pure statistical core of the input
// latency bench harness. Runnable WITHOUT a running app:
//   node --test tools/tests/lib/bench-stats.test.mjs

import { test } from "node:test";
import assert from "node:assert/strict";
import {
    percentile, mean, stdev, summarize, aggregateRuns,
    compareToBaseline, verdictExitCode, VERDICT,
} from "./bench-stats.mjs";

test("percentile: floored-index + edge cases", () => {
    assert.equal(percentile([], 95), null);
    assert.equal(percentile([5], 50), 5);
    assert.equal(percentile([1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 50), 6);
    assert.equal(percentile([1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 100), 10);
    assert.equal(percentile([10, 1, 5], 0), 1); // sorts first
});

test("mean / stdev", () => {
    assert.equal(mean([2, 4, 6]), 4);
    assert.equal(stdev([5]), null);            // <2 samples
    assert.ok(Math.abs(stdev([2, 4, 6]) - 1.632993) < 1e-5);
});

test("summarize: empty + populated", () => {
    const empty = summarize([]);
    assert.equal(empty.n, 0);
    assert.equal(empty.p95, null);
    const s = summarize([10, 12, 11, 13, 50]);
    assert.equal(s.n, 5);
    assert.equal(s.max, 50);
    assert.equal(s.p50, 12);
});

test("aggregateRuns: pooled + per-run p95 + spread (hardcoded math)", () => {
    const runs = [
        [10, 11, 12, 13, 14],
        [10, 11, 12, 13, 40], // one slow run
        [10, 11, 12, 13, 15],
    ];
    const agg = aggregateRuns(runs);
    assert.equal(agg.runs, 3);
    assert.equal(agg.pooled.n, 15);
    // per-run P95 = floored index 4 of 5 (the last element)
    assert.deepEqual(agg.runP95s, [14, 40, 15]);
    // median of [14,40,15] → sorted [14,15,40], floored idx 1 → 15
    assert.equal(agg.medianP95, 15);
    assert.equal(agg.p95Spread, 26); // 40 - 14
    // CoV = stdev([14,40,15]) / mean(23) = 12.0277 / 23 ≈ 0.5229
    assert.ok(Math.abs(agg.p95CoV - 0.5229) < 1e-3, `CoV was ${agg.p95CoV}`);
});

test("aggregateRuns: filters empty runs", () => {
    const agg = aggregateRuns([[], [1, 2, 3], null, undefined]);
    assert.equal(agg.runs, 1);
});

test("compareToBaseline: pass within tolerance", () => {
    const agg = aggregateRuns([[10, 10, 10, 10, 11]]); // medianP95 ~11
    const cmp = compareToBaseline(agg, { metric: "medianP95", value: 10 }, { tolerancePct: 20, maxCoV: 1 });
    assert.equal(cmp.verdict, VERDICT.PASS);
    assert.ok(cmp.deltaPct <= 20);
});

test("compareToBaseline: regress beyond tolerance", () => {
    const agg = aggregateRuns([[20, 20, 20, 20, 30]]); // medianP95 ~30
    const cmp = compareToBaseline(agg, { metric: "medianP95", value: 10 }, { tolerancePct: 20, maxCoV: 5 });
    assert.equal(cmp.verdict, VERDICT.REGRESS);
    assert.ok(cmp.deltaPct > 20);
});

test("compareToBaseline: improvement", () => {
    const agg = aggregateRuns([[5, 5, 5, 5, 6]]);
    const cmp = compareToBaseline(agg, { metric: "medianP95", value: 10 }, { tolerancePct: 20, maxCoV: 5 });
    assert.equal(cmp.verdict, VERDICT.IMPROVE);
});

test("compareToBaseline: no baseline", () => {
    const agg = aggregateRuns([[10, 11, 12]]);
    assert.equal(compareToBaseline(agg, null).verdict, VERDICT.NO_BASELINE);
});

test("compareToBaseline: noisy runner flagged before verdict", () => {
    // Wildly varying per-run P95s → high CoV → NOISY regardless of baseline.
    const runs = [
        [5, 5, 5, 5, 5],
        [50, 50, 50, 50, 50],
        [5, 5, 5, 5, 5],
    ];
    const agg = aggregateRuns(runs);
    const cmp = compareToBaseline(agg, { metric: "medianP95", value: 10 }, { tolerancePct: 20, maxCoV: 0.25 });
    assert.equal(cmp.verdict, VERDICT.NOISY);
});

test("verdictExitCode: report never blocks; gate blocks on regress/noisy", () => {
    assert.equal(verdictExitCode(VERDICT.REGRESS, "report"), 0);
    assert.equal(verdictExitCode(VERDICT.NOISY, "report"), 0);
    assert.equal(verdictExitCode(VERDICT.REGRESS, "gate"), 1);
    assert.equal(verdictExitCode(VERDICT.NOISY, "gate"), 1);
    assert.equal(verdictExitCode(VERDICT.PASS, "gate"), 0);
    assert.equal(verdictExitCode(VERDICT.IMPROVE, "gate"), 0);
    assert.equal(verdictExitCode(VERDICT.NO_BASELINE, "gate"), 0);
});
