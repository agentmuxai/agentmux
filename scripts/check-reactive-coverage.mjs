#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-reactive-coverage.mjs: CI ratchet on views that read srv once and never hear of a change.
//
// docs/reports/REPORT_REACTIVE_COVERAGE_AUDIT_2026_11_03.md: the Connectors panel did not update when an
// account was connected because it fetched on mount and listened to nothing. Of the 36 view files that
// fetch over RPC on mount, 24 had no live source in the file.
//
// THE RULE. A file under frontend/app/view that fetches over RPC when it mounts (`onMount` or
// `createResource` plus a `RpcApi.<X>Command(` call) needs a live source in the same file: an srv event
// (`onMuxEvent`, `WpsEvent.`), a poll (`pollWhileVisible`, `setInterval`), a store subscription, or a
// domain store (`createDomainStore`, `.follow()`, frontend/app/store/domain-store.ts). A file that
// should read once, such as a form that loads its options when it opens, says so with a line
//
//     // reactive-coverage: one-shot <why>
//
// The files that already break the rule are in scripts/reactive-coverage.baseline, one path per line.
// A listed file that gains a live source (or a marker, or goes away) must leave the list: `--update`
// does that. A file that is not listed may not break the rule, so the list only shrinks. The check is
// by file, as a first filter: it cannot tell a live source for one fetch from one for another, and
// the report says which views still need a real look.
//
// Usage:
//   node scripts/check-reactive-coverage.mjs            check
//   node scripts/check-reactive-coverage.mjs --update   drop the baseline entries that are fixed
//                                                        (never adds)
//   node scripts/check-reactive-coverage.mjs --init     write today's state as the baseline

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { isMain } from "./lib/is-main.mjs";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const BASELINE_FILE = "scripts/reactive-coverage.baseline";
const VIEW_DIR = "frontend/app/view/";

const SOURCE = /\.(ts|tsx)$/;
const NOT_A_VIEW = /\.(test|spec|bench|stories)\.[jt]sx?$|\.d\.ts$/;
const FETCH_ON_MOUNT = /\b(onMount|createResource)\s*\(/;
const RPC = /RpcApi\.\w+Command\(/;
const LIVE =
    /\bonMuxEvent\b|\bpollWhileVisible\b|\bsetInterval\s*\(|\bsubscribe\w*\s*\(|\buseMuxEvent\b|\bWpsEvent\.|\bcreateDomainStore\b|\.follow\(\)/;
const ONE_SHOT = /^[ \t]*\/\/[ \t]*reactive-coverage:[ \t]*one-shot[ \t]+\S/m;

// ── What counts ─────────────────────────────────────────────────────────────

/** Whether a repo-relative path (forward slashes) is a view source file. */
export function isViewPath(path) {
    return path.startsWith(VIEW_DIR) && SOURCE.test(path) && !NOT_A_VIEW.test(path);
}

/** Whether the text fetches over RPC on mount, and has no live source and no one-shot marker. */
export function isStale(text) {
    if (!FETCH_ON_MOUNT.test(text) || !RPC.test(text)) return false;
    return !LIVE.test(text) && !ONE_SHOT.test(text);
}

// ── The baseline file ───────────────────────────────────────────────────────

/** One path per line; `#` comments and blank lines ignored. */
export function parseBaseline(text) {
    const out = new Set();
    for (const raw of text.split("\n")) {
        const line = raw.trim();
        if (line && !line.startsWith("#")) out.add(line);
    }
    return out;
}

const HEADER = `# View files that fetch over RPC on mount with no live source in the file
# (scripts/check-reactive-coverage.mjs). A file here may only leave: give it a live source
# (a domain store, an srv event, a poll) and run \`node scripts/check-reactive-coverage.mjs --update\`.
# Adding a file is a hand edit that the PR has to justify.
`;

export function formatBaseline(paths) {
    return HEADER + [...paths].sort().join("\n") + "\n";
}

// ── Compare ─────────────────────────────────────────────────────────────────

/** `stale` is the set of view files breaking the rule now; returns who is new and who is fixed. */
export function compare(stale, baseline) {
    const added = [...stale].filter((p) => !baseline.has(p)).sort();
    const fixed = [...baseline].filter((p) => !stale.has(p)).sort();
    return { added, fixed };
}

/** The baseline after `--update`: the entries still breaking the rule. Never adds. */
export function lowered(stale, baseline) {
    return new Set([...baseline].filter((p) => stale.has(p)));
}

// ── Measure ─────────────────────────────────────────────────────────────────

/** The view files in the working tree that fetch on mount and have no live source. */
export function measure() {
    const r = spawnSync("git", ["ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", VIEW_DIR], {
        cwd: REPO_ROOT,
        maxBuffer: 1 << 26,
    });
    if (r.status !== 0) throw new Error(`git ls-files failed: ${r.stderr}`);
    const stale = new Set();
    for (const path of new Set(r.stdout.toString().split("\0"))) {
        if (!path || !isViewPath(path)) continue;
        const full = join(REPO_ROOT, path);
        if (!existsSync(full)) continue;
        if (isStale(readFileSync(full, "utf8"))) stale.add(path);
    }
    return stale;
}

// ── Main ────────────────────────────────────────────────────────────────────

if (isMain(import.meta.url)) {
    const stale = measure();
    const baselinePath = join(REPO_ROOT, BASELINE_FILE);

    if (process.argv.includes("--init")) {
        writeFileSync(baselinePath, formatBaseline(stale));
        console.log(`check-reactive-coverage: wrote ${BASELINE_FILE} (${stale.size} files).`);
        process.exit(0);
    }

    const baseline = parseBaseline(readFileSync(baselinePath, "utf8"));

    if (process.argv.includes("--update")) {
        const next = lowered(stale, baseline);
        writeFileSync(baselinePath, formatBaseline(next));
        console.log(`check-reactive-coverage: wrote ${BASELINE_FILE} (${next.size} files).`);
        process.exit(compare(stale, next).added.length ? 1 : 0);
    }

    const { added, fixed } = compare(stale, baseline);
    if (added.length) {
        console.error("ERROR: these view files fetch over RPC on mount and never hear of a change:");
        for (const p of added) console.error(`  ${p}`);
        console.error("Read the data through a domain store (frontend/app/store/domain-store.ts), which reloads on the");
        console.error("srv event that means it changed, or subscribe to that event or poll in the file.");
        console.error("A file that should read once says so: `// reactive-coverage: one-shot <why>`.");
        console.error();
    }
    if (fixed.length) {
        console.error(`ERROR: ${BASELINE_FILE} is out of date. These files are live now (thank you):`);
        for (const p of fixed) console.error(`  ${p}`);
        console.error(`Run \`node scripts/check-reactive-coverage.mjs --update\` and commit ${BASELINE_FILE},`);
        console.error("so they can't go quiet again.");
        console.error();
    }
    if (added.length || fixed.length) process.exit(1);

    console.log(`check-reactive-coverage: ok. ${baseline.size} view files still fetch once and listen to nothing.`);
}
