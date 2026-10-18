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
// A run that isn't the load it was asked for is refused, not reported: wrong
// number of agents or panes, agents that never played, turns still being
// played at the end, a transcript store that grew too little. The lab then
// starts over (--attempts, default 2).
//
// Compare two runs with compare.mjs. Build the portable with `task package`
// (a production build: a `task dev` build inflates every number).

import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { generateConversation } from "./lib/conversation.mjs";
import { connect, installScript, launch, layoutFor, portableExe } from "./lib/driver.mjs";
import { osSample, portBelongsTo, takeSample } from "./lib/sample.mjs";
import { agentsWithoutTurns, groupSample, inMegabytes, steady, table, transcriptGrowthProblem } from "./lib/summary.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, "..", "..", "..");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** A run that isn't the load it was asked for: not an error of the app, so the lab starts it over. */
class InvalidRun extends Error {}

function parseArgs(argv) {
    const a = { panes: 4, rounds: 10, port: 9333, every: 5, gapS: 15, settleS: 20, samples: 3, seed: 1, attempts: 2, out: "memory-lab-out", keep: false };
    const num = new Set(["panes", "rounds", "port", "every", "gapS", "settleS", "samples", "seed", "attempts"]);
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

/** The transcript store's size: its database and write-ahead log (the log grows with every write, so it shows turns the database hasn't absorbed yet). */
function transcriptBytes(home) {
    const db = join(home, "shared", "agents", "transcripts", "filestore.db");
    const size = (p) => (existsSync(p) ? statSync(p).size : 0);
    return size(db) + size(`${db}-wal`);
}

const appMB = (sample) => Math.round((groupSample(sample).app.private / 1048576) * 10) / 10;

/**
 * The layout must be what was asked for: a run with extra or missing agents
 * measures a different load. Counts the composers on screen, the agents made,
 * and the agent definitions srv holds (an agent made twice leaves a second one).
 */
async function requireLayout(driver, args) {
    const composers = await driver.eval(`document.querySelectorAll("textarea.agent-input").length`);
    const defs = await driver.labAgentNames();
    if (composers !== args.panes || driver.agents.length !== args.panes || defs.length !== args.panes) {
        throw new InvalidRun(`${composers} agent panes on screen, ${driver.agents.length} agents made and ${defs.length} agent definitions, expected ${args.panes} of each: ${defs.join(", ")}`);
    }
}

/**
 * Every agent must have played the first round's turn: a run whose agents never
 * ran reports a plausible, far too low number (an idle window). Gives a loaded
 * machine up to a minute more.
 */
async function requireTurns(driver, args) {
    let missing = [];
    for (let waited = 0; waited <= 60; waited += 5) {
        missing = agentsWithoutTurns(await driver.toolRows());
        if (missing.length === 0) return;
        await sleep(5000);
    }
    throw new InvalidRun(`${missing.length} of ${args.panes} agents never played their turn (no tool rows in their transcripts): ${missing.join(", ")}`);
}

/** The last turns must be over before the final is taken, or a slow build is measured with fewer turns played. */
async function requireIdle(driver, timeoutS = 300) {
    let quiet = 0;
    for (let waited = 0; waited <= timeoutS; waited += 5) {
        quiet = (await driver.busy()) === 0 ? quiet + 1 : 0;
        if (quiet >= 3) return;
        await sleep(5000);
    }
    throw new InvalidRun(`agents were still playing turns ${timeoutS} s after the last round`);
}

/** One try: starts the instance, runs the load, measures, writes the result. */
async function attempt(args, ctx) {
    const { exe, memtree, dir, home, script, layout } = ctx;
    const instance = await launch({ exe, home, port: args.port, script, log: join(dir, "instance.log") });
    const curve = [];
    let driver;
    // The instance has its own process group on macOS and Linux, so Ctrl-C reaches only the lab: end the instance first.
    const interrupted = async () => {
        await instance.stop();
        process.exit(130);
    };
    process.once("SIGINT", interrupted);
    process.once("SIGTERM", interrupted);
    try {
        if (!(await portBelongsTo(args.port, memtree, instance.pid))) {
            throw new Error(`CDP port ${args.port} isn't answered by the instance the lab started (the instance took another port, or something else answers there)`);
        }
        installScript(home, script);
        driver = await connect(args.port, home, join(dir, "work"));
        await driver.setupAccount();
        await driver.build(layout);
        await sleep(15_000);
        await requireLayout(driver, args);
        const transcriptsBefore = transcriptBytes(home);

        const mark = async (round) => {
            const s = await osSample(memtree, instance.pid, args.port);
            const g = groupSample(s);
            const row = { round, appMB: appMB(s), groupsMB: Object.fromEntries(Object.entries(g.groups).filter(([, v]) => v.procs).map(([k, v]) => [k, Math.round((v.private / 1048576) * 10) / 10])) };
            curve.push(row);
            console.log(`  round ${String(round).padStart(3)}: app ${row.appMB} MB  ${Object.entries(row.groupsMB).map(([k, v]) => `${k} ${v}`).join("  ")}`);
        };
        await mark(0);
        for (let r = 1; r <= args.rounds; r++) {
            await driver.send();
            await sleep(args.gapS * 1000);
            if (r === 1) await requireTurns(driver, args);
            if (r % args.every === 0 || r === args.rounds) await mark(r);
        }

        // Let the last turns finish and the frontend go idle, then measure with a forced GC.
        await requireIdle(driver);
        await sleep(args.settleS * 1000);
        const final = await takeSample({ port: args.port, memtree, rootPid: instance.pid, label: `${args.label}-final`, gc: true });
        const samples = [final];
        for (let i = 1; i < args.samples; i++) {
            await sleep(2000);
            samples.push(await osSample(memtree, instance.pid, args.port));
        }
        const summary = inMegabytes(steady(samples));

        // The store of record: the agents' turns must be in it, whatever the page shows.
        const turnBytes = JSON.parse(readFileSync(script, "utf8")).turns.slice(0, args.rounds).map((t) => JSON.stringify(t).length);
        const problem = transcriptGrowthProblem({
            before: transcriptsBefore,
            after: transcriptBytes(home),
            agents: args.panes,
            rounds: args.rounds,
            bytesPerTurn: turnBytes.reduce((a, b) => a + b, 0) / turnBytes.length,
        });
        if (problem) throw new InvalidRun(problem);

        const notes = [];
        if (process.platform === "darwin") notes.push("macOS: the totals are per-process footprints summed, an upper bound; footprint's own total over the app's processes is in final.footprint");
        const result = {
            label: args.label,
            when: new Date().toISOString(),
            platform: process.platform,
            portable: args.portable ?? null,
            layout: { panes: args.panes, tabs: layout },
            agents: driver.agents.map((a) => ({ name: a.name, id: a.id, tabId: a.tabId })),
            toolRows: await driver.toolRows(),
            rounds: args.rounds,
            gapSeconds: args.gapS,
            seed: args.seed,
            notes,
            curve,
            final,
            steady: summary,
        };
        writeFileSync(join(dir, "result.json"), JSON.stringify(result, null, 1));
        const md = [`# ${args.label}`, "", `${args.panes} agents over ${layout.length} tab(s), ${args.rounds} rounds (${args.gapS} s apart), forced GC, median of ${summary.samples} samples.`, "", table(summary), ...notes.map((n) => `\n> ${n}`)].join("\n");
        writeFileSync(join(dir, "summary.md"), md + "\n");
        console.log("\n" + md);
        console.log(`\n-> ${join(dir, "result.json")}`);
    } finally {
        process.off("SIGINT", interrupted);
        process.off("SIGTERM", interrupted);
        driver?.close();
        if (!args.keep) await instance.stop();
        else console.log(`left running: pid ${instance.pid}, CDP port ${args.port}`);
    }
}

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
    for (let n = 1; ; n++) {
        try {
            await attempt(args, { exe, memtree, dir, home, script, layout });
            return;
        } catch (e) {
            if (!(e instanceof InvalidRun)) throw e;
            writeFileSync(join(dir, `invalid-run-${n}.txt`), e.message + "\n");
            if (n >= args.attempts) throw new Error(`the run was not the load asked for, ${n} times: ${e.message} (see ${dir})`);
            console.log(`this run was not the load asked for (${e.message}); starting over (${n + 1} of ${args.attempts})`);
        }
    }
}

main().catch((e) => {
    console.error(e.message);
    process.exit(1);
});
