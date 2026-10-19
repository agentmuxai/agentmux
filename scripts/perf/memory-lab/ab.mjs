#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// An interleaved A/B run of the memory lab, then the comparison.
//
//   node scripts/perf/memory-lab/ab.mjs --a-portable <A> --b-portable <B> --runs 4 --out ab-out \
//        -- --panes 12 --rounds 10 --disk-load 50
//
// A or B may differ by build (--a-portable/--b-portable), by an srv swapped into the same portable (--a-srv,
// --b-srv, with --a-srv-commit/--b-srv-commit), by an environment variable (--a-env NAME=value, --b-env) or by an app
// setting (--a-setting NAME=value, --b-setting; written to the instance's settings.json). Everything after the
// lone -- goes to both sides' runs. The runs alternate A B A B ...; the comparison (medians, ranges, flags) is
// written to <out>/compare.md and <out>/ab.json lists what ran.

import { spawn } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { compareArgs, outcomeOf, parseAbArgs, planRuns, runArgs } from "./lib/abplan.mjs";

const here = dirname(fileURLToPath(import.meta.url));

/** Runs a node script to its end with its output passed through; resolves with its exit code. */
function runNode(script, args) {
    return new Promise((resolveRun) => {
        const child = spawn(process.execPath, [script, ...args], { stdio: "inherit" });
        child.on("exit", (code) => resolveRun(code ?? 1));
        child.on("error", () => resolveRun(1));
    });
}

async function main() {
    const spec = parseAbArgs(process.argv.slice(2));
    spec.out = resolve(spec.out);
    mkdirSync(spec.out, { recursive: true });
    const plan = planRuns(spec);
    console.log(`A/B: ${plan.map((r) => r.label).join(" ")}`);
    const ran = [];
    const finished = new Set();
    for (const run of plan) {
        console.log(`\n=== ${run.label} (${ran.length + 1} of ${plan.length}) ===`);
        const started = Date.now();
        const code = await runNode(join(here, "run.mjs"), runArgs(run, spec));
        const ok = code === 0 && existsSync(join(spec.out, run.label, "result.json"));
        if (ok) finished.add(run.label);
        ran.push({ ...run, exitCode: code, ok, seconds: Math.round((Date.now() - started) / 1000) });
    }
    const outcome = outcomeOf(plan, finished);
    writeFileSync(join(spec.out, "ab.json"), JSON.stringify({ spec: { ...spec, common: spec.common }, ran, outcome }, null, 1));
    const failed = ran.filter((r) => !r.ok).map((r) => r.label);
    if (failed.length) console.log(`\nruns that did not finish: ${failed.join(", ")}`);
    if (!outcome.comparable) {
        console.error("no run finished on at least one side: nothing to compare");
        process.exit(1);
    }
    const code = await runNode(join(here, "compare.mjs"), compareArgs(spec, outcome));
    process.exit(code);
}

main().catch((e) => {
    console.error(e.message);
    process.exit(2);
});
