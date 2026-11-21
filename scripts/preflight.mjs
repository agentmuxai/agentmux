#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// preflight: every pre-push gate CI runs on a PR that needs no build, in one command, one line per gate.
//
//   node scripts/preflight.mjs                 run every gate against this branch
//   node scripts/preflight.mjs --only a,b      only these gates (ids, from --list)
//   node scripts/preflight.mjs --list          the gates and when each runs
//   node scripts/preflight.mjs --all           run tsc and the generated-types checks whatever changed
//   node scripts/preflight.mjs --rust          also `cargo check --tests` on the CEF-free crates (the cargo shim queues it)
//   node scripts/preflight.mjs --jobs 6        how many light gates run at once (default 4)
//   node scripts/preflight.mjs --no-slot       run tsc and the generated-types checks without the build slot
//
// WHAT IT CHECKS. The branch as it will be committed: what is committed, staged or edited against the merge-base with
// origin/main (scripts/lib/branch-changes.mjs), so the answer is the same before and after `git commit`. A file git does
// not track yet is not on the branch until `git add`; the summary names those files so a new spec or changeset is not
// silently skipped. The light gates run in parallel. tsc and the generated-types checks run one after another under ONE
// ticket of the host's build slot, and only when the branch changes what they read.
//
// WHAT IT PRINTS. One line per gate, then a summary, then the first failure's output. A failing gate's whole output is in
// the log folder the summary names. The exit code is 1 when any gate failed.
import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { baseRef, changedRows, pathsOf, untrackedFiles } from "./lib/branch-changes.mjs";
import { isMain } from "./lib/is-main.mjs";
import { GATES, HEAVY_GATES, RUST_PACKAGES } from "./lib/preflight-gates.mjs";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const GATE_TIMEOUT_MS = 30 * 60 * 1000;
const MARK = "@@RESULT ";

/** Command-line options. */
export function parseArgs(argv) {
    const o = { only: null, list: false, all: false, rust: false, jobs: 4, slot: true, heavyInner: null, help: false };
    for (let i = 0; i < argv.length; i++) {
        const a = argv[i];
        if (a === "--only") o.only = String(argv[++i] ?? "").split(",").filter(Boolean);
        else if (a === "--jobs") o.jobs = Math.max(1, Number(argv[++i]) || 4);
        else if (a === "--heavy-inner") o.heavyInner = String(argv[++i] ?? "").split(",").filter(Boolean);
        else if (a === "--list") o.list = true;
        else if (a === "--all") o.all = true;
        else if (a === "--rust") o.rust = true;
        else if (a === "--no-slot") o.slot = false;
        else if (a === "--help" || a === "-h") o.help = true;
        else throw new Error(`unknown argument ${a} (see --help)`);
    }
    return o;
}

const stripAnsi = (s) => s.replace(/\x1b\[[0-9;]*m/g, "");

/** The line that says why a gate failed: the first one that names a failure, else the last one. */
export function reasonOf(output) {
    const lines = stripAnsi(output).split(/\r?\n/).map((l) => l.trim()).filter(Boolean);
    const hit = lines.find((l) => /\b(FAIL|FAILED|STALE|error|violation|not allowed)\b|✗/i.test(l)) ?? lines[lines.length - 1] ?? "no output";
    return hit.length > 150 ? `${hit.slice(0, 147)}...` : hit;
}

/** Run one command from the repository root; resolves to `{ ok, ms, out }` and never rejects. */
function runCommand(cmd, timeoutMs = GATE_TIMEOUT_MS) {
    return new Promise((resolve) => {
        const t0 = Date.now();
        let out = "";
        let child;
        try {
            child = spawn(cmd[0], cmd.slice(1), { cwd: ROOT, windowsHide: true, env: process.env });
        } catch (e) {
            resolve({ ok: false, ms: 0, out: `cannot run ${cmd[0]}: ${e.message}` });
            return;
        }
        const timer = setTimeout(() => {
            out += `\nstopped after ${Math.round(timeoutMs / 60000)} minutes`;
            child.kill();
        }, timeoutMs);
        child.stdout.on("data", (d) => (out += d));
        child.stderr.on("data", (d) => (out += d));
        child.on("error", (e) => {
            clearTimeout(timer);
            resolve({ ok: false, ms: Date.now() - t0, out: `cannot run ${cmd[0]}: ${e.message}` });
        });
        child.on("close", (code) => {
            clearTimeout(timer);
            resolve({ ok: code === 0, ms: Date.now() - t0, out });
        });
    });
}

async function runPool(items, jobs, fn) {
    const results = new Array(items.length);
    let next = 0;
    const worker = async () => {
        while (next < items.length) {
            const at = next++;
            results[at] = await fn(items[at]);
        }
    };
    await Promise.all(Array.from({ length: Math.min(jobs, items.length) }, worker));
    return results;
}

/**
 * The changeset gate (.github/workflows/changeset.yml): the branch adds one `.changesets/<ts>-<slug>.md` with a
 * description. A release PR, which consumes changesets, needs none. Pure, so it can be tested.
 */
export function changesetVerdict(rows, untracked, readText) {
    const isChangeset = (p) => /^\.changesets\/[0-9]+-[^/]+\.md$/.test(p);
    if (rows.some((r) => r[0][0] === "D" && isChangeset(r[r.length - 1]))) return { ok: true, note: "release PR: changesets consumed" };
    const added = pathsOf(rows, "A").filter(isChangeset);
    if (added.length === 0) {
        const loose = untracked.filter(isChangeset);
        const reason = loose.length
            ? `${loose[0]} is not added to git: run git add ${loose[0]}`
            : 'no changeset: run scripts/changeset.sh <patch|minor|major> "<one-line summary>" and git add the file (or label the PR no-changeset)';
        return { ok: false, reason };
    }
    for (const f of added) {
        const text = readText(f) ?? "";
        const body = text.replace(/^---[\s\S]*?\n---\s*/, "").trim();
        if (!body) return { ok: false, reason: `${f} has no description after the front matter` };
    }
    return { ok: true };
}

/** Whether a heavy gate is worth running for these changed paths. */
export function heavyWanted(gate, paths, all) {
    return all || paths.some((p) => gate.paths.some((prefix) => p.startsWith(prefix)));
}

const seconds = (ms) => `${(ms / 1000).toFixed(1)}s`;

function printLine(r) {
    const mark = r.skipped ? "skip" : r.ok ? "ok" : "FAIL";
    const tail = r.skipped ? `  (${r.note})` : r.ok ? `  ${seconds(r.ms)}` : `  ${seconds(r.ms)}  ${r.reason}`;
    console.log(`  ${mark.padEnd(5)} ${r.name.padEnd(40)}${tail}`);
}

function finish(gate, run) {
    const r = { id: gate.id, name: gate.name, ok: run.ok, ms: run.ms, out: run.out, reason: run.ok ? "" : reasonOf(run.out) };
    printLine(r);
    return r;
}

function slotScript() {
    const p = path.join(os.homedir(), ".agentmux", "shared", "locks", "build-slot.sh");
    return existsSync(p) ? p : null;
}

/** Run the heavy gates one after another in this process, printing one machine-readable line each. */
async function runHeavyInner(ids) {
    for (const gate of HEAVY_GATES.filter((g) => ids.includes(g.id))) {
        const run = await runCommand(gate.cmd);
        console.log(MARK + JSON.stringify({ id: gate.id, ok: run.ok, ms: run.ms, out: run.out.slice(-6000) }));
    }
}

/** Run the heavy gates under one ticket of the build slot (or directly with --no-slot or no slot script). */
async function runHeavy(gates, opts) {
    const slot = opts.slot ? slotScript() : null;
    if (!slot) {
        const out = [];
        for (const gate of gates) out.push(finish(gate, await runCommand(gate.cmd)));
        return out;
    }
    console.log(`  ....  waiting for the build slot once, then ${gates.map((g) => g.id).join(", ")}`);
    const self = fileURLToPath(import.meta.url);
    const run = await runCommand(["bash", slot, "--reason", "preflight: tsc and generated types", "--", "node", self, "--heavy-inner", gates.map((g) => g.id).join(",")], 3 * 60 * 60 * 1000);
    const byId = new Map();
    for (const line of run.out.split(/\r?\n/)) {
        if (line.startsWith(MARK)) {
            const j = JSON.parse(line.slice(MARK.length));
            byId.set(j.id, j);
        }
    }
    return gates.map((gate) =>
        finish(gate, byId.get(gate.id) ?? { ok: false, ms: 0, out: `the build slot did not run it: ${reasonOf(run.out)}` }),
    );
}

function readWorking(p) {
    try {
        return readFileSync(path.join(ROOT, p), "utf8");
    } catch {
        return null;
    }
}

function printUntracked(untracked) {
    if (untracked.length === 0) return;
    const shown = untracked.slice(0, 8).join(", ");
    console.log(`\n  ${untracked.length} untracked file(s) are not checked; git add the ones this PR includes: ${shown}${untracked.length > 8 ? ", ..." : ""}`);
}

function printFirstFailure(results, logDir) {
    const first = results.find((r) => !r.ok && !r.skipped);
    if (!first) return;
    mkdirSync(logDir, { recursive: true });
    for (const r of results.filter((x) => !x.ok && !x.skipped)) writeFileSync(path.join(logDir, `${r.id}.log`), r.out);
    const lines = stripAnsi(first.out).split(/\r?\n/).filter((l) => l.trim()).slice(-14);
    console.log(`\nfirst failure: ${first.name} (${first.id})`);
    for (const l of lines) console.log(`  ${l}`);
    console.log(`\nfull output of each failed gate: ${logDir}`);
}

function help() {
    console.log(readFileSync(fileURLToPath(import.meta.url), "utf8").split("\n").filter((l) => l.startsWith("//")).slice(2, 12).map((l) => l.slice(3)).join("\n"));
}

async function main() {
    const opts = parseArgs(process.argv.slice(2));
    if (opts.help) return help();
    if (opts.heavyInner) return runHeavyInner(opts.heavyInner);
    if (opts.list) {
        for (const g of GATES) console.log(`${g.id.padEnd(28)} ${g.name}`);
        for (const g of HEAVY_GATES) console.log(`${g.id.padEnd(28)} ${g.name}  (build slot; when the branch changes ${g.paths.join(", ")})`);
        console.log(`${"changeset".padEnd(28)} the branch adds a changeset`);
        return;
    }
    process.chdir(ROOT);
    const ref = baseRef(ROOT);
    const rows = ref ? changedRows({ root: ROOT, ref }) : null;
    if (rows === null) {
        console.log("preflight: cannot find the base branch (git fetch origin main); the gates that compare with it would skip.");
    }
    const paths = pathsOf(rows ?? []);
    const untracked = untrackedFiles(ROOT);
    const head = execFileSync("git", ["rev-parse", "--short", ref ?? "HEAD"], { cwd: ROOT, encoding: "utf8" }).trim();
    console.log(`preflight: ${paths.length} file(s) changed against ${ref ?? "HEAD"} (${head}); light gates run ${opts.jobs} at a time\n`);

    const want = (id) => !opts.only || opts.only.includes(id);
    const results = [];
    const cs = changesetVerdict(rows ?? [], untracked, readWorking);
    if (want("changeset")) results.push(finish({ id: "changeset", name: "the branch adds a changeset" }, { ok: cs.ok, ms: 0, out: cs.reason ?? "" }));
    results.push(...(await runPool(GATES.filter((g) => want(g.id)), opts.jobs, (g) => runCommand(g.cmd).then((run) => finish(g, run)))));

    const heavy = HEAVY_GATES.filter((g) => want(g.id));
    const [run, skip] = [heavy.filter((g) => heavyWanted(g, paths, opts.all)), heavy.filter((g) => !heavyWanted(g, paths, opts.all))];
    for (const g of skip) {
        const r = { id: g.id, name: g.name, ok: true, skipped: true, note: "the branch changes nothing it reads; --all runs it" };
        printLine(r);
        results.push(r);
    }
    if (run.length) results.push(...(await runHeavy(run, opts)));
    if (opts.rust && want("cargo-check") && paths.some((p) => p.startsWith("crates/") || p.startsWith("Cargo."))) {
        const gate = { id: "cargo-check", name: "cargo check --tests (CEF-free crates)", cmd: ["cargo", "check", "--tests", ...RUST_PACKAGES.flatMap((p) => ["-p", p])] };
        results.push(finish(gate, await runCommand(gate.cmd, 2 * 60 * 60 * 1000)));
    }

    const failed = results.filter((r) => !r.ok && !r.skipped);
    const skipped = results.filter((r) => r.skipped);
    console.log(`\npreflight: ${results.length - failed.length - skipped.length} ok, ${failed.length} failed, ${skipped.length} skipped`);
    printUntracked(untracked);
    printFirstFailure(results, path.join(os.tmpdir(), "agentmux-preflight", String(process.pid)));
    process.exitCode = failed.length ? 1 : 0;
}

if (isMain(import.meta.url)) {
    main().catch((e) => {
        console.error(`preflight: ${e.message}`);
        process.exit(2);
    });
}
