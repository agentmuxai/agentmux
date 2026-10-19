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

import { compareRuns, DEFAULT_MARGINS, runsTable } from "./lib/summary.mjs";

/** Why two results can't be compared, or null. */
export function mismatch(base, head) {
    const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);
    if (!same(base.layout, head.layout)) return `different layouts: ${JSON.stringify(base.layout)} vs ${JSON.stringify(head.layout)}`;
    if (base.rounds !== head.rounds) return `different rounds: ${base.rounds} vs ${head.rounds}`;
    if (base.seed !== head.seed) return `different conversations (seed ${base.seed} vs ${head.seed})`;
    // A run that recycled its window is a different load, and its final follows the recycle.
    if ((base.recycleAfter ?? 0) !== (head.recycleAfter ?? 0)) return `different recycles: after round ${base.recycleAfter ?? 0} vs ${head.recycleAfter ?? 0} (0 is none)`;
    if ((base.recycleAfter ?? 0) > 0 && base.recycleCommand !== head.recycleCommand) return `different recycle commands: ${base.recycleCommand} vs ${head.recycleCommand}`;
    // Asked for the same recycle, but one build didn't do it (it lacks the command, or refused): one final follows a recycle and the other doesn't.
    if ((base.recycleAfter ?? 0) > 0 && Boolean(base.recycle?.recycled) !== Boolean(head.recycle?.recycled)) return `only one of the runs recycled its window: ${base.recycle?.recycled ? head.label : base.label} did not (${base.recycle?.recycled ? head.recycle?.blocked : base.recycle?.blocked})`;
    if ((base.holdMinutes ?? 0) !== (head.holdMinutes ?? 0)) return `different idle holds: ${base.holdMinutes ?? 0} min vs ${head.holdMinutes ?? 0} min`;
    if ((base.workloadVersion ?? 1) !== (head.workloadVersion ?? 1)) return `different workload versions (${base.workloadVersion ?? 1} vs ${head.workloadVersion ?? 1}): the rounds are paced differently`;
    const loadOf = (r) => (r.diskLoad ? r.diskLoad.rateMBps : 0);
    if (loadOf(base) !== loadOf(head)) return `different background disk loads: ${loadOf(base) || "none"} vs ${loadOf(head) || "none"} MB/s`;
    if (base.settleSeconds !== head.settleSeconds) return `different settle delays: ${base.settleSeconds} s vs ${head.settleSeconds} s`;
    if (base.steady?.samples !== head.steady?.samples) return `different sample counts: ${base.steady?.samples} vs ${head.steady?.samples}`;
    if (base.gapSeconds !== head.gapSeconds) return `different round spacing: ${base.gapSeconds} s vs ${head.gapSeconds} s`;
    if (base.platform !== head.platform) return `different operating systems (${base.platform} vs ${head.platform}): each reports its own memory metric`;
    return null;
}

function main() {
    const argv = process.argv.slice(2);
    const take = (flag) => {
        const i = argv.indexOf(flag);
        return i >= 0 ? argv.splice(i, 2)[1] : null;
    };
    const md = take("--md");
    // Default margins differ per metric (DEFAULT_MARGINS); a flag sets one number for all CPU, or for the round time.
    const cpuFlag = take("--cpu-margin");
    const wallFlag = take("--wall-margin");
    const margins = { cpuPct: cpuFlag != null ? Number(cpuFlag) : DEFAULT_MARGINS.cpuPct, wallPct: wallFlag != null ? Number(wallFlag) : DEFAULT_MARGINS.wallPct };
    for (const [flag, v] of [["--cpu-margin", cpuFlag], ["--wall-margin", wallFlag]]) {
        if (v != null && !(Number.isFinite(Number(v)) && v.trim() !== "")) {
            console.error(`${flag} needs a number of percent (10, not "${v}")`);
            process.exit(2);
        }
    }
    // `<base.json> <head.json>`, or comma-separated lists of runs of each side (several runs, interleaved, tell noise from change).
    const [basePaths, headPaths] = [take("--base") ?? argv[0], take("--head") ?? argv[1]].map((x) => (x ? x.split(",") : []));
    if (!basePaths.length || !headPaths.length) {
        console.error("usage: compare.mjs <base result.json[,more]> <head result.json[,more]> [--md out.md] [--cpu-margin <pct for every group>] [--wall-margin <pct>]");
        process.exit(2);
    }
    const base = basePaths.map((p) => JSON.parse(readFileSync(p, "utf8")));
    const head = headPaths.map((p) => JSON.parse(readFileSync(p, "utf8")));
    // Every run must be comparable with the first of its side and with the other side.
    const all = [...base, ...head];
    for (const r of all.slice(1)) {
        const why = mismatch(all[0], r);
        if (why) {
            console.error(`not comparable: ${all[0].label} and ${r.label}: ${why}`);
            process.exit(2);
        }
    }
    const baseLabel = base.length > 1 ? `${base[0].label}..` : base[0].label;
    const headLabel = head.length > 1 ? `${head[0].label}..` : head[0].label;
    const out = [
        `## ${headLabel} against ${baseLabel}`,
        "",
        `${head[0].layout.panes} agents, ${head[0].rounds} rounds, forced GC, private working set (MB) and CPU seconds over the workload.`,
        "",
        runsTable(compareRuns(base, head, margins), baseLabel, headLabel),
    ].join("\n");
    console.log(out);
    if (md) writeFileSync(md, out + "\n");
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) main();
