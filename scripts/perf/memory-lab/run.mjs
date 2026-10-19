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
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { generateConversation } from "./lib/conversation.mjs";
import { connect, installScript, launch, layoutFor, portableExe } from "./lib/driver.mjs";
import { driftWarning, withReplacedSrv } from "./lib/srv.mjs";
import { runRecycle, recycleSaving } from "./lib/recycle.mjs";
import { agentProcessesCountedAsApp, cpuLedger, forcedGcWindows, osSample, portBelongsTo, takeSample } from "./lib/sample.mjs";
import { agentsWithoutTurns, cpuTable, groupSample, inMegabytes, steady, table, transcriptGrowthProblem, wallSummary } from "./lib/summary.mjs";
import { cpuBetween, writtenBetween } from "./lib/cpu.mjs";
import { storeLockOf } from "./lib/storelock.mjs";
import { parseDiskLoad, startDiskLoad } from "./lib/diskload.mjs";
import { volumeOf } from "./lib/volume.mjs";
import { slowSendsAgainstLongTasks, summariseLongTasks } from "./lib/longtasks.mjs";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, "..", "..", "..");
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/** What to add to a failure when the srv was replaced: a srv from another commit than the portable can launch and still break the app. */
function srvHint(args) {
    return args.srv ? ". The srv was replaced (--srv): an srv built from a different commit than the portable can launch yet leave the frontend unable to create agents or play turns; build both from one commit, with only the change under test differing" : "";
}

/** A run that isn't the load it was asked for: not an error of the app, so the lab starts it over. */
class InvalidRun extends Error {}

function parseArgs(argv) {
    const a = { panes: 4, rounds: 10, port: 9333, every: 5, gapS: 15, settleS: 20, samples: 3, seed: 1, attempts: 2, recycleAfter: 0, holdMinutes: 0, recycleCommand: "Reload Window (Free Memory)", out: "memory-lab-out", keep: false };
    const num = new Set(["panes", "rounds", "port", "every", "gapS", "settleS", "samples", "seed", "attempts", "recycleAfter", "holdMinutes", "diskLoadCapGb"]);
    const flags = { "--keep": "keep" };
    for (let i = 0; i < argv.length; i++) {
        const raw = argv[i];
        if (flags[raw]) {
            a[flags[raw]] = true;
            continue;
        }
        const key = raw.replace(/^--/, "").replace(/-([a-z])/g, (_, c) => c.toUpperCase());
        if (!raw.startsWith("--") || !(key in a || ["portable", "label", "memtree", "exe", "srv", "srvCommit", "diskLoad", "diskLoadCapGb"].includes(key))) throw new Error(`unknown argument ${raw}`);
        const value = argv[++i];
        if (value === undefined) throw new Error(`${raw} needs a value`);
        a[key] = num.has(key) ? Number(value) : value;
    }
    if (!a.portable && !a.exe) throw new Error("give --portable <dir> (a `task package` folder), or --exe <path>");
    a.diskLoadRate = parseDiskLoad(a.diskLoad);
    if (a.diskLoadCapGb !== undefined && !(Number.isFinite(a.diskLoadCapGb) && a.diskLoadCapGb > 0)) {
        throw new Error(`--disk-load-cap-gb needs a positive number of GB (got "${a.diskLoadCapGb}"): it is what keeps a stress run from wearing out the disk`);
    }
    if (a.diskLoadCapGb !== undefined && !a.diskLoadRate) throw new Error("--disk-load-cap-gb needs --disk-load");
    if (a.srv && (!a.portable || a.exe)) throw new Error("--srv replaces the srv of a --portable folder: give --portable and not --exe (an --exe has no srv beside it to replace, and would be launched instead of the copy)");
    if (a.recycleAfter !== 0 && !(Number.isInteger(a.recycleAfter) && a.recycleAfter >= 1 && a.recycleAfter <= a.rounds)) {
        throw new Error(`--recycle-after ${a.recycleAfter} is not a round of this run: a whole number from 1 to --rounds (${a.rounds}), or leave it out`);
    }
    if (a.holdMinutes < 0 || (a.holdMinutes > 0 && a.recycleAfter === 0)) throw new Error("--hold-minutes needs --recycle-after (use --recycle-command none for the control): the agents stop after that round and the window is left idle");
    a.label ??= basename(a.portable ?? dirname(a.exe));
    return a;
}

function memtreeBinary(given) {
    if (given) return given;
    const bin = join(repoRoot, "target", "release", "examples", process.platform === "win32" ? "memtree.exe" : "memtree");
    // Always built: cargo makes it a no-op when it is current, and a binary left from before a change to
    // memtree (it gained per-process CPU time) would measure without it.
    console.log("building memtree (crates/procstats) ...");
    execFileSync("cargo", ["build", "--release", "-p", "agentmux-procstats", "--example", "memtree"], { cwd: repoRoot, stdio: "inherit" });
    if (!existsSync(bin)) throw new Error(`${bin} was not built`);
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
    throw new InvalidRun(`${missing.length} of ${args.panes} agents never played their turn (no tool rows in their transcripts): ${missing.join(", ")}${srvHint(args)}`);
}

/**
 * How the workload is paced, stamped in every result: results of different
 * versions are different loads and `compare.mjs` refuses to set them side by side.
 * 1: a fixed gap after each send. 2: a round ends when its agents are idle (and
 * not before the gap), and its time is recorded.
 */
export const WORKLOAD_VERSION = 2;

/**
 * Sends one round and waits for it: the milliseconds from the send until every
 * agent is idle again (null when no agent was ever seen working, or they never
 * went idle within `capS`), and at least `gapS` after the send either way.
 */
async function playRound(driver, args, capS = Math.max(180, args.gapS * 8), sendLog = null, round = null) {
    const t0 = Date.now();
    const timings = await driver.send();
    if (sendLog) timings.forEach((t, i) => sendLog.push({ round, agent: i, ...t }));
    let sawWorking = false;
    let quiet = 0;
    let doneAt = null;
    while (Date.now() - t0 < capS * 1000) {
        await sleep(500);
        const busy = await driver.busy().catch(() => null);
        if (busy == null) continue;
        if (busy > 0) {
            sawWorking = true;
            quiet = 0;
        } else if (sawWorking && ++quiet === 1) {
            doneAt = Date.now() - t0; // idle for the first time; two polls (1 s) confirm it
        } else if (sawWorking && quiet >= 2) break;
        if (!sawWorking && Date.now() - t0 > 10_000) break; // never seen working: nothing to time
    }
    const took = sawWorking && quiet >= 2 ? doneAt : null;
    const rest = args.gapS * 1000 - (Date.now() - t0);
    if (rest > 0) await sleep(rest);
    return took;
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

/** What a recycle did, in a few lines. */
function reportRecycle(rec) {
    if (rec.control) {
        console.log("    control: nothing pressed, same arming and samples");
        console.log(`    before: app ${rec.before.appMB} MB, renderers ${rec.before.rendererMB} MB (${rec.before.rendererProcs}), documents ${rec.before.documents}`);
        for (const m of rec.after) console.log(`    +${String(m.atSeconds).padStart(2)} s: app ${m.appMB} MB, renderers ${m.rendererMB} MB (${m.rendererProcs}), documents ${m.documents}`);
        return;
    }
    if (!rec.recycled) {
        console.log(`  the command did not recycle the window: ${rec.blocked}`);
        console.log("  (the build must have the command: \"Reload Window (Free Memory)\", #5086; on a build without it the palette finds nothing)");
        return;
    }
    const row = (m) => `    +${String(m.atSeconds).padStart(2)} s: app ${m.appMB} MB, renderers ${m.rendererMB} MB (${m.rendererProcs}), documents ${m.documents}`;
    console.log(`    before: app ${rec.before.appMB} MB, renderers ${rec.before.rendererMB} MB (${rec.before.rendererProcs}), documents ${rec.before.documents}`);
    for (const m of rec.after) console.log(row(m));
    const s = recycleSaving(rec);
    const scroll = rec.state.scroll.notArmed ? "not tested (could not be armed)" : rec.state.scroll.pass ? "kept" : "LOST";
    console.log(`    saved: app ${s.appMB} MB, renderers ${s.rendererMB} MB; draft ${rec.state.draft.pass ? "kept" : "LOST"}, scroll position ${scroll}`);
}

/** After the recycle (or its control) the agents stop: the window's memory once a minute, with no GC forced, for `holdMinutes`. */
async function holdIdle(args, memtree, rootPid) {
    const rows = [];
    for (let minute = 1; minute <= args.holdMinutes; minute++) {
        await sleep(60_000);
        const s = await osSample(memtree, rootPid, args.port);
        const g = groupSample(s);
        const mb = (b) => (b == null ? null : Math.round((b / 1048576) * 10) / 10);
        rows.push({ minute, appMB: mb(g.app.private), rendererMB: mb(g.groups.renderer.private), srvMB: mb(g.groups.srv.private) });
        console.log(`    idle +${String(minute).padStart(2)} min: app ${rows.at(-1).appMB} MB, renderers ${rows.at(-1).rendererMB} MB, srv ${rows.at(-1).srvMB} MB`);
    }
    return rows;
}

/** One try: starts the instance, runs the load, measures, writes the result. */
async function attempt(args, ctx) {
    const { exe, memtree, dir, home, script, layout, swapped } = ctx;
    const instance = await launch({ exe, home, port: args.port, script, log: join(dir, "instance.log") });
    const curve = [];
    let diskLoad = null; // stopped in the outer finally on any path out of this attempt: a retry must not start a second writer beside it
    cpuLedger.reset();
    forcedGcWindows.length = 0;
    let driver;
    let succeeded = false;
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
        try {
            await driver.build(layout);
            await sleep(15_000);
            await requireLayout(driver, args);
        } catch (e) {
            if (args.srv && e instanceof Error) e.message += srvHint(args);
            throw e;
        }
        const transcriptsBefore = transcriptBytes(home);

        const mark = async (round) => {
            const s = await osSample(memtree, instance.pid, args.port);
            const strays = agentProcessesCountedAsApp(s.procs);
            if (strays.length) {
                throw new Error(`${strays.length} agent CLI process(es) are counted as the app (${[...new Set(strays.map((p) => p.name))].join(", ")}): the app total would include the agents. Add the process name to FAKE_CLI_ROOTS in lib/sample.mjs.`);
            }
            const g = groupSample(s);
            const row = { round, appMB: appMB(s), groupsMB: Object.fromEntries(Object.entries(g.groups).filter(([, v]) => v.procs).map(([k, v]) => [k, Math.round((v.private / 1048576) * 10) / 10])) };
            curve.push(row);
            console.log(`  round ${String(round).padStart(3)}: app ${row.appMB} MB  ${Object.entries(row.groupsMB).map(([k, v]) => `${k} ${v}`).join("  ")}`);
        };
        await mark(0);
        // Said at once, not after the run: memtree reports CPU time of every process; none at all means a memtree without it (--memtree <old binary>), not an idle app.
        if (cpuLedger.totals().app === 0) throw new Error("no process reported CPU time: the memtree in use predates per-process CPU (give none to have the lab build the current one)");
        // The CPU of the workload is what is spent after this point; startup and building the layout are before it.
        const cpuAtStart = cpuLedger.totals();
        const writtenAtStart = cpuLedger.writtenTotals();
        const storageAtStart = cpuLedger.storageTotals();
        const roundMs = [];
        const roundWindows = [];
        const sendLog = [];
        await driver.installLongTasks();
        const volumeAtStart = volumeOf(dir);
        // The background disk load starts with the workload and ends with its last turn, before the settle and the forced-GC final.
        diskLoad = args.diskLoadRate ? startDiskLoad({ dir, mbps: args.diskLoadRate, capGB: args.diskLoadCapGb }) : null;
        let diskLoadResult = null;
        let recycle = null;
        // With --hold-minutes the agents stop at the recycle round: what matters then is how long the window stays down.
        const playedRounds = args.holdMinutes > 0 ? args.recycleAfter : args.rounds;
        let hold = null;
        for (let r = 1; r <= playedRounds; r++) {
            const windowStart = Date.now();
            roundMs.push(await playRound(driver, args, undefined, sendLog, r));
            roundWindows.push({ round: r, start: windowStart, end: Date.now() });
            if (r === 1) await requireTurns(driver, args);
            if (r % args.every === 0 || r === playedRounds) await mark(r);
            if (r === args.recycleAfter) {
                console.log(`  recycling the window after round ${r}: ${args.recycleCommand}`);
                recycle = await runRecycle({ driver, memtree, port: args.port, rootPid: instance.pid, label: args.label, command: args.recycleCommand });
                reportRecycle(recycle);
                if (args.holdMinutes > 0) hold = await holdIdle(args, memtree, instance.pid);
            }
        }

        // Let the last turns finish and the frontend go idle, then measure with a forced GC.
        try {
            await requireIdle(driver);
        } finally {
            if (diskLoad) diskLoadResult = await diskLoad.stop();
        }
        await sleep(args.settleS * 1000);
        const final = await takeSample({ port: args.port, memtree, rootPid: instance.pid, label: `${args.label}-final`, gc: true });
        const samples = [final];
        for (let i = 1; i < args.samples; i++) {
            await sleep(2000);
            samples.push(await osSample(memtree, instance.pid, args.port));
        }
        const summary = inMegabytes(steady(samples));
        const cpu = { fromRound: 0, seconds: cpuBetween(cpuAtStart, cpuLedger.totals()), note: "highest CPU time seen per process, summed per group; includes the settle and the forced-GC final" };
        const wall = wallSummary(roundMs);
        // A group with a process whose counter was never readable has no total (null, not a short sum or "0 MB"); with no counter
        // anywhere (a memtree without it, or an OS that would not say) there is no `io` at all.
        const incomplete = cpuLedger.writeIncomplete();
        const writtenMB = writtenBetween(writtenAtStart, cpuLedger.writtenTotals());
        for (const g of incomplete) writtenMB[g] = null;
        // The bytes that reached storage: the same, null on Windows (no such counter) and for a group missing a counter.
        const storageIncomplete = cpuLedger.storageIncomplete();
        const storageWrittenMB = writtenBetween(storageAtStart, cpuLedger.storageTotals());
        for (const g of storageIncomplete) storageWrittenMB[g] = null;
        const hasStorage = cpuLedger.sawStorageCounters();
        const io = !Object.values(writtenMB).some((v) => v != null) ? null : { fromRound: 0, writtenMB, incompleteGroups: incomplete, storageWrittenMB: hasStorage ? storageWrittenMB : null, note: "highest write counter seen per process, summed per group (Windows and Linux: bytes passed to write calls; macOS: bytes written to disk). storageWrittenMB: the part that reached storage (Linux write_bytes less cancelled_write_bytes, macOS disk bytes; none on Windows)" };
        const rawLong = await driver.longTasks().catch(() => null);
        const pageLongTasks = summariseLongTasks(rawLong, roundWindows, forcedGcWindows);
        const sendsVsLong = slowSendsAgainstLongTasks(sendLog, rawLong);

        // The store of record: the agents' turns must be in it, whatever the page shows.
        const turnBytes = JSON.parse(readFileSync(script, "utf8")).turns.slice(0, playedRounds).map((t) => JSON.stringify(t).length);
        const problem = transcriptGrowthProblem({
            before: transcriptsBefore,
            after: transcriptBytes(home),
            agents: args.panes,
            rounds: playedRounds,
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
            srvReplaced: swapped?.replaced ?? null,
            layout: { panes: args.panes, tabs: layout },
            agents: driver.agents.map((a) => ({ name: a.name, id: a.id, tabId: a.tabId })),
            toolRows: await driver.toolRows(),
            // What was asked; `playedRounds` is how many were played (fewer with --hold-minutes).
            workloadVersion: WORKLOAD_VERSION,
            diskLoad: diskLoadResult,
            // How full the volume was (see lib/volume.mjs): a stall on a nearly full disk is not the same finding as one on an empty disk.
            volume: { atStart: volumeAtStart, atEnd: volumeOf(dir) },
            rounds: args.rounds,
            gapSeconds: args.gapS,
            settleSeconds: args.settleS,
            recycleAfter: args.recycleAfter,
            recycleCommand: args.recycleAfter ? args.recycleCommand : null,
            holdMinutes: args.holdMinutes,
            playedRounds,
            hold,
            seed: args.seed,
            notes,
            curve,
            recycle: recycle ? { ...recycle, afterRound: args.recycleAfter, saving: recycleSaving(recycle) } : null,
            final,
            steady: summary,
            cpu,
            io,
            wall,
            // The slowest page command of the run: a machine near the driver's limit (90 s) shows here before it fails.
            slowestEvalMs: driver.slowestEvalMs(),
            // How long srv's transcript store kept callers waiting (issue #5141): from srv's own warnings, null without a log.
            storeLock: storeLockOf(home),
            // The page's main thread (tasks of 50 ms or more) and each send's time, to tell a busy page from a waiting srv.
            pageLongTasks,
            sendsVsLongTasks: sendsVsLong,
            sends: sendLog.map((s) => ({ ...s, start: new Date(s.start).toISOString() })),
            slowEvals: driver.slowEvals(), // { total, slowest }: every command that took 5 s or more, the 30 slowest listed
        };
        writeFileSync(join(dir, "result.json"), JSON.stringify(result, null, 1));
        const md = [`# ${args.label}`, "", `${args.panes} agents over ${layout.length} tab(s), ${playedRounds} rounds (${args.gapS} s apart)${args.holdMinutes > 0 ? `, then held idle ${args.holdMinutes} min` : ""}, forced GC, median of ${summary.samples} samples.`, "", table(summary), "", cpuTable(cpu, wall, io), ...(pageLongTasks ? ["", `Page main thread: ${pageLongTasks.count} long task(s) of 50 ms or more, ${pageLongTasks.totalMs} ms in all, longest ${pageLongTasks.maxMs} ms, ${pageLongTasks.over1s} over 1 s; ${sendsVsLong.slowSends} send(s) took over 5 s, ${sendsVsLong.overlappingLongTask} of them while a long task of 500 ms or more ran.${pageLongTasks.truncated ? ` Only the first ${pageLongTasks.recorded} long tasks were timed: the totals, rounds and overlaps cover those.` : ""}`] : []), ...(volumeAtStart ? ["", `Volume under the data folder: ${volumeAtStart.freePct}% free (${volumeAtStart.freeGB} of ${volumeAtStart.totalGB} GB) at the start.`] : []), ...(result.storeLock ? ["", `Store lock: longest hold of srv's transcript store connection ${result.storeLock.longestHeldMs} ms${result.storeLock.longestHeldSite ? ` (${result.storeLock.longestHeldSite})` : ""}, ${result.storeLock.holds} hold(s) and ${result.storeLock.waits} wait(s) of 500 ms or more.`] : []), ...notes.map((n) => `\n> ${n}`)].join("\n");
        writeFileSync(join(dir, "summary.md"), md + "\n");
        console.log("\n" + md);
        console.log(`\n-> ${join(dir, "result.json")}`);
        succeeded = true;
    } finally {
        process.off("SIGINT", interrupted);
        process.off("SIGTERM", interrupted);
        if (diskLoad) await diskLoad.stop(); // a no-op when the workload already stopped it
        driver?.close();
        // --keep leaves a good run's instance for inspection; a run about to be retried must free its port.
        if (!(args.keep && succeeded)) await instance.stop();
        else console.log(`left running: pid ${instance.pid}, CDP port ${args.port}`);
    }
}

async function main() {
    const args = parseArgs(process.argv.slice(2));
    const memtree = memtreeBinary(args.memtree);
    const dir = resolve(args.out, args.label);
    const home = join(dir, "home");
    mkdirSync(dir, { recursive: true });
    // Nothing of an earlier run under this label may stand in for this one: a run that fails leaves no report.
    for (const old of ["result.json", "summary.md", ...readdirSync(dir).filter((f) => /^invalid-run-\d+\.txt$/.test(f))]) rmSync(join(dir, old), { force: true });
    // --srv: the portable with its srv swapped for the one under test (a copy; the original is untouched).
    const swapped = args.srv ? withReplacedSrv(args.portable, resolve(args.srv), dir) : null;
    if (swapped) {
        console.log(`srv replaced: ${swapped.replaced.file} <- ${swapped.replaced.from} (${swapped.replaced.sha256})`);
        swapped.replaced.srvCommit = args.srvCommit ?? null;
        const drift = driftWarning(args.portable, args.srvCommit);
        if (drift) console.log(`  WARNING: ${drift}`);
        else if (!args.srvCommit) console.log("  (give --srv-commit <sha> to have the lab check it against the portable's commit; the srv must be built from the same commit as the portable)");
    }
    const exe = args.exe ?? portableExe(swapped ? swapped.portable : args.portable);
    if (!existsSync(exe)) throw new Error(`${exe} not found`);


    // The same conversation for every build: a seed fixes it.
    const script = join(dir, "conversation.json");
    writeFileSync(script, JSON.stringify(generateConversation({ turns: args.rounds + 2, seed: args.seed })));

    const layout = layoutFor(args.panes);
    console.log(`${args.label}: ${args.panes} agents over ${layout.length} tab(s) [${layout.join("/")}], ${args.holdMinutes > 0 ? args.recycleAfter : args.rounds} rounds${args.holdMinutes > 0 ? ` then ${args.holdMinutes} min idle` : ""}, port ${args.port}`);
    for (let n = 1; ; n++) {
        try {
            await attempt(args, { exe, memtree, dir, home, script, layout, swapped });
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
