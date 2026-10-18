#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The memory lab: measures a production build's memory under a repeatable
// agent load (docs/reports/REPORT_RENDERER_MEMORY_ROOT_CAUSES_2026_10_12.md §2,
// scripts/perf/memory-lab/README.md).
//
//   node scripts/perf/memory-lab/run.mjs --portable <dir> [--label <name>]
//        [--panes 4|12] [--rounds 10] [--out <dir>] [--port 9333]
//
// It starts the portable isolated (its own data folder, its own CDP port),
// makes the scripted agents (4 in one tab, or 12 over 4 tabs), has every agent
// play one scripted, tool-heavy turn per round, and measures: the OS's private
// working set and commit per process group as the rounds go (the curve), then,
// after the agents settle, a Chromium memory-infra dump with a forced garbage
// collection plus OS samples (the final, whose median is the steady value).
// Writes <out>/<label>/result.json and summary.md, and prints the table.
//
// Compare two runs with compare.mjs. Build the portable with `task package`
// (a production build: a `task dev` build inflates every number).

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { generateConversation } from "./lib/conversation.mjs";
import { connect, installScript, launch, layoutFor, portableExe } from "./lib/driver.mjs";
import { osSample, portBelongsTo, takeSample } from "./lib/sample.mjs";
import { groupSample, inMegabytes, steady, table } from "./lib/summary.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, "..", "..", "..");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function parseArgs(argv) {
    const a = { panes: 4, rounds: 10, port: 9333, every: 5, gapS: 15, settleS: 20, samples: 3, seed: 1, out: "memory-lab-out", keep: false };
    const num = new Set(["panes", "rounds", "port", "every", "gapS", "settleS", "samples", "seed"]);
    const flags = { "--keep": "keep" };
    for (let i = 0; i < argv.length; i++) {
        const raw = argv[i];
        if (flags[raw]) {
            a[flags[raw]] = true;
            continue;
        }
        const key = raw.replace(/^--/, "").replace(/-([a-z])/g, (_, c) => c.toUpperCase());
        if (!raw.startsWith("--") || !(key in a || ["portable", "label", "memtree", "exe"].includes(key))) throw new Error(`unknown argument ${raw}`);
        const value = argv[++i];
        if (value === undefined) throw new Error(`${raw} needs a value`);
        a[key] = num.has(key) ? Number(value) : value;
    }
    if (!a.portable && !a.exe) throw new Error("give --portable <dir> (a `task package` folder), or --exe <path>");
    a.label ??= basename(a.portable ?? dirname(a.exe));
    return a;
}

function memtreeBinary(given) {
    if (given) return given;
    const bin = join(repoRoot, "target", "release", "examples", process.platform === "win32" ? "memtree.exe" : "memtree");
    if (!existsSync(bin)) {
        console.log("building memtree (crates/procstats) ...");
        execFileSync("cargo", ["build", "--release", "-p", "agentmux-procstats", "--example", "memtree"], { cwd: repoRoot, stdio: "inherit" });
    }
    return bin;
}

const appMB = (sample) => Math.round((groupSample(sample).app.private / 1048576) * 10) / 10;

async function main() {
    const args = parseArgs(process.argv.slice(2));
    const exe = args.exe ?? portableExe(args.portable);
    if (!existsSync(exe)) throw new Error(`${exe} not found`);
    const memtree = memtreeBinary(args.memtree);
    const dir = resolve(args.out, args.label);
    const home = join(dir, "home");
    mkdirSync(dir, { recursive: true });

    // The same conversation for every build: a seed fixes it.
    const script = join(dir, "conversation.json");
    writeFileSync(script, JSON.stringify(generateConversation({ turns: args.rounds + 2, seed: args.seed })));

    const layout = layoutFor(args.panes);
    console.log(`${args.label}: ${args.panes} agents over ${layout.length} tab(s) [${layout.join("/")}], ${args.rounds} rounds, port ${args.port}`);
    const instance = await launch({ exe, home, port: args.port, script, log: join(dir, "instance.log") });
    const curve = [];
    let driver;
    try {
        if (!(await portBelongsTo(args.port, memtree, instance.pid))) {
            throw new Error(`CDP port ${args.port} isn't answered by the instance the lab started (the instance took another port, or something else answers there)`);
        }
        installScript(home, script);
        driver = await connect(args.port, home, join(dir, "work"));
        await driver.setupAccount();
        await driver.build(layout);
        await sleep(15_000);

        const mark = (round) => {
            const s = osSample(memtree, instance.pid);
            const g = groupSample(s);
            const row = { round, appMB: appMB(s), groupsMB: Object.fromEntries(Object.entries(g.groups).filter(([, v]) => v.procs).map(([k, v]) => [k, Math.round((v.private / 1048576) * 10) / 10])) };
            curve.push(row);
            console.log(`  round ${String(round).padStart(3)}: app ${row.appMB} MB  ${Object.entries(row.groupsMB).map(([k, v]) => `${k} ${v}`).join("  ")}`);
        };
        mark(0);
        for (let r = 1; r <= args.rounds; r++) {
            await driver.send();
            await sleep(args.gapS * 1000);
            if (r % args.every === 0 || r === args.rounds) mark(r);
        }

        // Let the last turns finish and the frontend go idle, then measure with a forced GC.
        await sleep(args.settleS * 1000);
        const final = await takeSample({ port: args.port, memtree, rootPid: instance.pid, label: `${args.label}-final`, gc: true });
        const samples = [final];
        for (let i = 1; i < args.samples; i++) {
            await sleep(2000);
            samples.push(osSample(memtree, instance.pid));
        }
        const summary = inMegabytes(steady(samples));

        const result = {
            label: args.label,
            when: new Date().toISOString(),
            platform: process.platform,
            portable: args.portable ?? null,
            layout: { panes: args.panes, tabs: layout },
            rounds: args.rounds,
            gapSeconds: args.gapS,
            seed: args.seed,
            curve,
            final,
            steady: summary,
        };
        writeFileSync(join(dir, "result.json"), JSON.stringify(result, null, 1));
        const md = [`# ${args.label}`, "", `${args.panes} agents over ${layout.length} tab(s), ${args.rounds} rounds (${args.gapS} s apart), forced GC, median of ${summary.samples} samples.`, "", table(summary)].join("\n");
        writeFileSync(join(dir, "summary.md"), md + "\n");
        console.log("\n" + md);
        console.log(`\n-> ${join(dir, "result.json")}`);
    } finally {
        driver?.close();
        if (!args.keep) instance.stop();
        else console.log(`left running: pid ${instance.pid}, CDP port ${args.port}`);
    }
}

main().catch((e) => {
    console.error(e.message);
    process.exit(1);
});
