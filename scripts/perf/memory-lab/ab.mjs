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
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { compareArgs, outcomeOf, parseAbArgs, planRuns, runArgs, sideGivenUp, unrunnableSides } from "./lib/abplan.mjs";

const here = dirname(fileURLToPath(import.meta.url));

/** Runs a node script to its end with its output passed through; resolves with its exit code. */
function runNode(script, args) {
    return new Promise((resolveRun) => {
        const child = spawn(process.execPath, [script, ...args], { stdio: "inherit" });
        child.on("exit", (code) => resolveRun(code ?? 1));
        child.on("error", () => resolveRun(1));
    });
}

/** Why a run failed, in the lab's own words: the message of its last invalid-run file (written for each attempt). */
function failureOf(dir) {
    try {
        const n = (f) => parseInt(f.slice("invalid-run-".length), 10);
        const files = readdirSync(dir).filter((f) => /^invalid-run-\d+\.txt$/.test(f)).sort((x, y) => n(x) - n(y));
        return files.length ? readFileSync(join(dir, files.at(-1)), "utf8").trim() : null;
    } catch {
        return null;
    }
}

async function main() {
    const spec = parseAbArgs(process.argv.slice(2));
    spec.out = resolve(spec.out);
    mkdirSync(spec.out, { recursive: true });
    const plan = planRuns(spec);
    console.log(`A/B: ${plan.map((r) => r.label).join(" ")}`);
    const ran = [];
    const finished = new Set();
    const reasons = {};
    const skipped = [];
    for (const run of plan) {
        // A side whose runs keep failing is not tried again: the rest would take as long to fail the same way.
        if (sideGivenUp(run, ran)) {
            skipped.push(run.label);
            console.log(`\n=== ${run.label} not run: ${ran.filter((r) => r.side === run.side).length} runs of side ${run.side.toUpperCase()} failed and none finished ===`);
            continue;
        }
        console.log(`\n=== ${run.label} (${ran.length + 1} of ${plan.length}) ===`);
        const started = Date.now();
        const code = await runNode(join(here, "run.mjs"), runArgs(run, spec));
        const ok = code === 0 && existsSync(join(spec.out, run.label, "result.json"));
        if (ok) finished.add(run.label);
        else reasons[run.label] = failureOf(join(spec.out, run.label)) ?? `the run ended with exit code ${code} and wrote no result`;
        ran.push({ ...run, exitCode: code, ok, seconds: Math.round((Date.now() - started) / 1000) });
    }
    const outcome = outcomeOf(plan, finished);
    writeFileSync(join(spec.out, "ab.json"), JSON.stringify({ spec: { ...spec, common: spec.common }, plan, ran, outcome, reasons, skipped }, null, 1));
    const failed = ran.filter((r) => !r.ok).map((r) => r.label);
    if (failed.length) console.log(`\nruns that did not finish: ${failed.join(", ")}`);
    if (!outcome.comparable) {
        for (const line of unrunnableSides(plan, outcome, reasons, skipped)) console.error(line);
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
