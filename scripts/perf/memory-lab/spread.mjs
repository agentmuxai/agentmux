#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// How much the measures vary between repeated runs of one build (Phase 0 of the nightly performance check:
// sizing the job and the noise).
//
//   node scripts/perf/memory-lab/spread.mjs <result.json> [<result.json> ...] [--wall 421,415,430] [--md out.md]

import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { spreadMarkdown } from "./lib/measures.mjs";

function main() {
    const argv = process.argv.slice(2);
    const take = (flag) => {
        const i = argv.indexOf(flag);
        return i >= 0 ? argv.splice(i, 2)[1] : null;
    };
    const wall = (take("--wall") ?? "").split(",").filter(Boolean).map(Number);
    const md = take("--md");
    if (!argv.length) {
        console.error("usage: spread.mjs <result.json> [<result.json> ...] [--wall seconds,seconds] [--md out.md]");
        process.exit(2);
    }
    const results = argv.map((p) => JSON.parse(readFileSync(p, "utf8")));
    const out = `${results.length} run(s) of ${results[0].layout?.panes} agents, ${results[0].rounds} rounds.\n\n${spreadMarkdown(results, wall)}`;
    console.log(out);
    if (md) writeFileSync(md, out + "\n");
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) main();
