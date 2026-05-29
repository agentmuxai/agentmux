#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// bench-aggregate.mjs — run an input-latency bench N times and judge it against
// a committed baseline (delta-vs-baseline), not a flaky absolute threshold.
//
// Input-first Phase 0.1 (discussion #1161). Wraps the existing benches
// (bench-agent-keystroke.mjs, bench-term-echo.mjs) without modifying them:
// runs each N times, extracts the headline metric per run, aggregates the
// per-run distribution, and emits a REPORT (never blocks) or GATE (blocks on
// regression / excessive run-to-run noise) verdict.
//
// Why: see tools/tests/lib/bench-stats.mjs header. Single-run absolute gates
// false-positive on variable hardware; median-of-N + delta-vs-baseline +
// reporting-mode-first is the design the input-first reviewers converged on.
//
//   PREREQUISITE: a running AgentMux instance with the relevant pane open
//   (agent pane for `agent`, terminal for `term`) on a PINNED reference device.
//   The numbers are only trustworthy on stable hardware — see
//   tools/tests/baselines/README.md.
//
// Usage:
//   node tools/tests/bench-aggregate.mjs --bench agent --runs 5 \
//        --baseline tools/tests/baselines/agent-keystroke.<device>.json \
//        --mode report -- --cdp-port 9223 --count 200
//
//   node tools/tests/bench-aggregate.mjs --bench agent --runs 5 \
//        --baseline <path> --update-baseline --device "thinkpad-x250"
//
// Everything after `--` is passed through to the underlying bench verbatim.

import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import {
    percentile, mean, stdev,
    loadBaseline, saveBaseline, compareToBaseline, verdictExitCode, formatVerdict, fmtMs,
} from "./lib/bench-stats.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));

const PRESETS = {
    agent: { script: "bench-agent-keystroke.mjs", metricPath: "keystroke.stats.p95", label: "agent-keystroke P95" },
    term: { script: "bench-term-echo.mjs", metricPath: "busy.p95", label: "term-echo busy P95" },
};

// ── CLI ─────────────────────────────────────────────────────────────────────
const argv = process.argv.slice(2);
const ddIdx = argv.indexOf("--");
const ownArgs = ddIdx === -1 ? argv : argv.slice(0, ddIdx);
const passthrough = ddIdx === -1 ? [] : argv.slice(ddIdx + 1);

function opt(name, fallback) {
    const i = ownArgs.indexOf(name);
    return i !== -1 ? ownArgs[i + 1] : fallback;
}
function flag(name) { return ownArgs.includes(name); }

if (flag("--help") || ownArgs.length === 0) {
    console.log(`
node tools/tests/bench-aggregate.mjs --bench <agent|term> [options] -- [bench passthrough args]

  --bench <agent|term>    Which bench to run (required)
  --runs <n>              Number of repetitions (default: 5)
  --baseline <path>       Baseline JSON to compare against (and to --update-baseline)
  --metric-path <dotpath> Override the per-run metric path (default per preset)
  --mode <report|gate>    report = never blocks (default); gate = exit 1 on regress/noisy
  --tolerance-pct <n>     Regression threshold vs baseline (default: 20)
  --max-cov <f>           Max run-to-run coefficient of variation before 'noisy' (default: 0.25)
  --update-baseline       Write the measured medianP95 to --baseline and exit 0
  --device <name>         Device label stored in an updated baseline
  --help

Presets:  agent → ${PRESETS.agent.script} (${PRESETS.agent.metricPath})
          term  → ${PRESETS.term.script}  (${PRESETS.term.metricPath})

Prereq: a running AgentMux on a pinned reference device with the pane open.
`);
    process.exit(ownArgs.length === 0 ? 2 : 0);
}

const benchName = opt("--bench");
const preset = PRESETS[benchName];
if (!preset) {
    console.error(`bench-aggregate: --bench must be one of ${Object.keys(PRESETS).join(", ")}`);
    process.exit(2);
}
const RUNS = parseInt(opt("--runs", "5"), 10);
const BASELINE_PATH = opt("--baseline");
const METRIC_PATH = opt("--metric-path", preset.metricPath);
const MODE = opt("--mode", "report");
const TOLERANCE_PCT = parseFloat(opt("--tolerance-pct", "20"));
const MAX_COV = parseFloat(opt("--max-cov", "0.25"));
const UPDATE_BASELINE = flag("--update-baseline");
const DEVICE = opt("--device", "unspecified-device");

// ── Helpers ─────────────────────────────────────────────────────────────────
function getPath(obj, dotPath) {
    return dotPath.split(".").reduce((o, k) => (o == null ? undefined : o[k]), obj);
}

function runOnce(i) {
    const tmp = mkdtempSync(join(tmpdir(), "agm-bench-"));
    const out = join(tmp, "run.json");
    const script = join(HERE, preset.script);
    const res = spawnSync("node", [script, ...passthrough, "--output-file", out], {
        encoding: "utf8",
        stdio: ["ignore", "inherit", "inherit"],
    });
    let value = null;
    try {
        const json = JSON.parse(readFileSync(out, "utf8"));
        value = getPath(json, METRIC_PATH);
    } catch (e) {
        console.error(`  run ${i + 1}: could not read metric '${METRIC_PATH}' from output (${e.message})`);
    } finally {
        rmSync(tmp, { recursive: true, force: true });
    }
    if (res.status !== 0) {
        console.error(`  run ${i + 1}: bench exited ${res.status} (continuing; the aggregate decides pass/fail)`);
    }
    return typeof value === "number" ? value : null;
}

// ── Main ────────────────────────────────────────────────────────────────────
console.log(`bench-aggregate: ${preset.label} — ${RUNS} runs, mode=${MODE}`);
const values = [];
for (let i = 0; i < RUNS; i++) {
    console.log(`── run ${i + 1}/${RUNS} ──────────────────────────────────────────`);
    const v = runOnce(i);
    if (v != null) {
        values.push(v);
        console.log(`  run ${i + 1} ${preset.label} = ${fmtMs(v)}`);
    }
}

if (values.length === 0) {
    console.error("bench-aggregate: no runs produced a metric value. Is AgentMux running with the pane open?");
    process.exit(2);
}

// Aggregate the per-run headline metric into the shape compareToBaseline wants.
const sd = stdev(values);
const m = mean(values);
const agg = {
    runs: values.length,
    runP95s: values,
    medianP95: percentile(values, 50),
    p95Spread: values.length > 1 ? Math.max(...values) - Math.min(...values) : 0,
    p95CoV: sd != null && m ? sd / m : null,
};

console.log("");
console.log("─".repeat(70));
console.log(`${preset.label}: per-run = [${values.map((v) => v.toFixed(1)).join(", ")}] ms`);
console.log(`  median=${fmtMs(agg.medianP95)}  spread=${fmtMs(agg.p95Spread)}  CoV=${agg.p95CoV == null ? "—" : (agg.p95CoV * 100).toFixed(1) + "%"}`);

if (UPDATE_BASELINE) {
    if (!BASELINE_PATH) {
        console.error("bench-aggregate: --update-baseline requires --baseline <path>");
        process.exit(2);
    }
    saveBaseline(BASELINE_PATH, {
        metric: "medianP95",
        value: agg.medianP95,
        device: DEVICE,
        bench: benchName,
        metricPath: METRIC_PATH,
        runs: agg.runs,
        capturedAt: new Date().toISOString(),
        note: "median of per-run P95. Recapture on the same pinned device after a deliberate perf change.",
    });
    console.log(`  ✓ baseline written to ${BASELINE_PATH} (medianP95=${fmtMs(agg.medianP95)}, device=${DEVICE})`);
    process.exit(0);
}

const baseline = loadBaseline(BASELINE_PATH);
const cmp = compareToBaseline(agg, baseline, { tolerancePct: TOLERANCE_PCT, maxCoV: MAX_COV });
console.log(formatVerdict(cmp, MODE));
console.log("─".repeat(70));

if (cmp.verdict === "no-baseline") {
    console.log("No baseline to compare against. Capture one with --update-baseline on the pinned device.");
}
process.exit(verdictExitCode(cmp.verdict, MODE));
