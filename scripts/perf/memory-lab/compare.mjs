#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Two memory-lab runs side by side: what a change saved (or cost), per process
// group and in total.
//
//   node scripts/perf/memory-lab/compare.mjs <base result.json> <head result.json> [--md out.md]
//
// Run both with the same --panes, --rounds and --seed (the comparison refuses
// otherwise): the same conversation on both builds is what makes the numbers
// comparable.

import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { compare, comparisonTable } from "./lib/summary.mjs";

/** Why two results can't be compared, or null. */
export function mismatch(base, head) {
    const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
    if (!same(base.layout, head.layout)) return `different layouts: ${JSON.stringify(base.layout)} vs ${JSON.stringify(head.layout)}`;
    if (base.rounds !== head.rounds) return `different rounds: ${base.rounds} vs ${head.rounds}`;
    if (base.seed !== head.seed) return `different conversations (seed ${base.seed} vs ${head.seed})`;
    return null;
}

function main() {
    const argv = process.argv.slice(2);
    const mdAt = argv.indexOf("--md");
    const md = mdAt >= 0 ? argv.splice(mdAt, 2)[1] : null;
    const [basePath, headPath] = argv;
    if (!basePath || !headPath) {
        console.error("usage: compare.mjs <base result.json> <head result.json> [--md out.md]");
        process.exit(2);
    }
    const base = JSON.parse(readFileSync(basePath, "utf8"));
    const head = JSON.parse(readFileSync(headPath, "utf8"));
    const why = mismatch(base, head);
    if (why) {
        console.error(`not comparable: ${why}`);
        process.exit(2);
    }
    const out = [
        `## ${head.label} against ${base.label}`,
        "",
        `${head.layout.panes} agents, ${head.rounds} rounds, forced GC, private working set (MB).`,
        "",
        comparisonTable(compare(base.steady, head.steady), base.label, head.label),
    ].join("\n");
    console.log(out);
    if (md) writeFileSync(md, out + "\n");
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) main();
