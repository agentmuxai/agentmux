#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The memory soak gate: one memory-lab result against the lines in budget.json.
//
//   node scripts/perf/memory-lab/gate.mjs <result.json> [--budget budget.json] [--md out.md]
//
// budget.json says which workload is gated (platform, panes, rounds, seed), the
// line for each metric in MB (null: no line, only reported), and the mode:
//   "report"  - prints the table and what WOULD fail; always exits 0
//   "enforce" - exits 1 when a metric is over its line, 2 when the result isn't
//               the gated workload (nothing was measured; that must not pass)
// The lines are the operator's to set: with every line null the gate cannot fail.

import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

/** The metrics gated, with where each is in a result's `steady`. */
const METRICS = [
    ["app", "App total (private working set)", (s) => s.app?.privateMB],
    ["renderer", "Renderers", (s) => s.groups?.renderer?.privateMB],
    ["srv", "srv", (s) => s.groups?.srv?.privateMB],
];

/** Why a result isn't the gated workload, or null. */
export function notTheWorkload(result, workload) {
    const got = {
        platform: result.platform,
        panes: result.layout?.panes,
        rounds: result.rounds,
        seed: result.seed,
        recycleAfter: result.recycleAfter ?? 0,
    };
    for (const [key, want] of Object.entries(workload)) {
        if (got[key] !== want) return `${key} is ${JSON.stringify(got[key])}, the gated workload has ${JSON.stringify(want)}`;
    }
    if (!result.steady?.app || typeof result.steady.app.privateMB !== "number") return "the result has no steady-state numbers";
    return null;
}

/**
 * @returns {{ rows: {metric:string, label:string, valueMB:number|null, limitMB:number|null, over:boolean}[], problem: string|null, over: boolean, exitCode: number }}
 */
export function evaluate(result, budget) {
    const problem = notTheWorkload(result, budget.workload ?? {});
    const rows = METRICS.map(([metric, label, pick]) => {
        const valueMB = problem ? null : (pick(result.steady) ?? null);
        const limitMB = budget.limitsMB?.[metric] ?? null;
        return { metric, label, valueMB, limitMB, over: valueMB != null && limitMB != null && valueMB > limitMB };
    });
    // A metric with a line that couldn't be read counts as over: a gate that passes on missing data isn't one.
    const unreadable = rows.some((r) => r.limitMB != null && r.valueMB == null);
    const over = rows.some((r) => r.over) || (!problem && unreadable);
    const enforce = budget.mode === "enforce";
    return { rows, problem, over, exitCode: !enforce ? 0 : problem || unreadable ? 2 : over ? 1 : 0 };
}

export function report(outcome, result, budget) {
    const f = (v) => (v == null ? "n/a" : `${v}`);
    const lines = [`## Memory soak gate (${budget.mode})`, ""];
    if (outcome.problem) lines.push(`**Not measured:** ${outcome.problem}.`, "");
    lines.push("| metric | MB | line (MB) | |", "|---|---:|---:|---|");
    for (const r of outcome.rows) {
        const verdict = r.limitMB == null ? "no line set" : r.valueMB == null ? "not read" : r.over ? (budget.mode === "enforce" ? "OVER" : "would be over") : "within";
        lines.push(`| ${r.label} | ${f(r.valueMB)} | ${f(r.limitMB)} | ${verdict} |`);
    }
    if (!outcome.problem) lines.push("", `${result.layout?.panes} agents, ${result.rounds} rounds, seed ${result.seed}, ${result.platform}; median of ${result.steady?.samples} forced-GC samples.`);
    if (budget.mode !== "enforce") lines.push("", "Report only: this gate fails nothing until the operator sets the lines and the mode.");
    return lines.join("\n");
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
    const args = process.argv.slice(2);
    const take = (flag, fallback) => {
        const i = args.indexOf(flag);
        if (i < 0) return fallback;
        const [, value] = args.splice(i, 2);
        return value;
    };
    const budgetPath = take("--budget", fileURLToPath(new URL("./budget.json", import.meta.url)));
    const mdPath = take("--md", null);
    const resultPath = args[0];
    if (!resultPath) {
        console.error("usage: gate.mjs <result.json> [--budget budget.json] [--md out.md]");
        process.exit(64);
    }
    const budget = JSON.parse(readFileSync(budgetPath, "utf8"));
    const result = JSON.parse(readFileSync(resultPath, "utf8"));
    const outcome = evaluate(result, budget);
    const md = report(outcome, result, budget);
    console.log(md);
    if (mdPath) writeFileSync(mdPath, md + "\n");
    process.exit(outcome.exitCode);
}
