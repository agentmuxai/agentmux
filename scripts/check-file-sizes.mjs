#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-file-sizes.mjs — CI ratchet on the size of source files.
//
// docs/specs/SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md: large files only
// get split if something stops them regrowing afterwards. `agent-view.tsx`
// grew by half after its first split because nothing held the line.
//
// THE RULES. A source file may have at most LIMIT lines, unless it is listed
// in scripts/check-file-sizes.baseline, which holds every file that was
// already over the limit. For a listed file:
//
//   - it may not grow past its baseline number;
//   - when it shrinks, its baseline number must come down with it, so the
//     room it gave up can't be taken back later. `--update` does that.
//
// A file that is not listed may not cross the limit, so the list only shrinks.
// Raising a number, or adding a file, is a hand edit of the baseline that
// the PR has to explain.
//
// WHAT COUNTS. Tracked (and untracked, not ignored) files with a code
// extension (SOURCE_EXT), except:
//   - tests: `*.test.*`, `*.spec.*`, `*.bench.*`; Rust files named `tests`,
//     or starting `test_`/`tests_`, or ending `_test`/`_tests` (RUST_TEST_NAME);
//     anything under a `tests/`, `test/`, `__tests__/`, `__mocks__/` or `e2e/`
//     directory;
//   - fixtures and snapshots: under `fixtures/`, `test-fixtures/` or
//     `__snapshots__/`, or with "fixture" in the file name;
//   - generated files and bindings: `*.d.ts`, the ts-rs bindings in
//     frontend/types/rpc/, and any file whose head says `@generated` or
//     `DO NOT EDIT`.
// Rust `#[cfg(test)]` modules inside a source file count toward it. Moving
// them to a sibling `tests.rs` is a fine way to make room.
//
// A line is a `\n`; a last line without one counts too.
//
// Usage:
//   node scripts/check-file-sizes.mjs            check
//   node scripts/check-file-sizes.mjs --update   lower the baseline to today's
//                                                sizes (never raises or adds)

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const BASELINE_FILE = "scripts/check-file-sizes.baseline";

export const LIMIT = 1500;

const SOURCE_EXT = /\.(rs|ts|tsx|js|jsx|mjs|cjs)$/;
const TEST_DIRS = new Set(["tests", "test", "__tests__", "__mocks__", "e2e"]);
const FIXTURE_DIRS = new Set(["fixtures", "test-fixtures", "__snapshots__"]);
const GENERATED_DIRS = ["frontend/types/rpc/"];
const TEST_NAME = /\.(test|spec|bench)\.[cm]?[jt]sx?$/;
const RUST_TEST_NAME = /^(tests|.+_tests|tests_.+|test_.+|.+_test)\.rs$/;
const GENERATED_MARK = /@generated|DO NOT EDIT/;

// ── What counts ─────────────────────────────────────────────────────────────

/** Whether a repo-relative path (forward slashes) is a source file by path alone. */
export function isSourcePath(path) {
    if (!SOURCE_EXT.test(path) || path.endsWith(".d.ts")) return false;
    if (GENERATED_DIRS.some((dir) => path.startsWith(dir))) return false;
    const parts = path.split("/");
    const name = parts.pop();
    if (parts.some((dir) => TEST_DIRS.has(dir) || FIXTURE_DIRS.has(dir))) return false;
    if (TEST_NAME.test(name) || /fixture/i.test(name)) return false;
    if (name.endsWith(".rs") && RUST_TEST_NAME.test(name)) return false;
    return true;
}

/** Whether a file's contents mark it as generated (checked in its first 1 KB). */
export function isGenerated(buf) {
    return GENERATED_MARK.test(buf.subarray(0, 1024).toString("utf8"));
}

/** Lines in a file: one per `\n`, plus a last line that lacks one. */
export function countLines(buf) {
    let n = 0;
    for (let i = buf.indexOf(10); i !== -1; i = buf.indexOf(10, i + 1)) n++;
    if (buf.length > 0 && buf[buf.length - 1] !== 10) n++;
    return n;
}

// ── The baseline file ───────────────────────────────────────────────────────

/** `<lines> <path>` per line; `#` comments and blank lines ignored. */
export function parseBaseline(text) {
    const out = new Map();
    for (const [i, raw] of text.split("\n").entries()) {
        const line = raw.trim();
        if (!line || line.startsWith("#")) continue;
        const m = /^(\d+)\s+(\S+)$/.exec(line);
        if (!m) throw new Error(`${BASELINE_FILE}:${i + 1}: expected "<lines> <path>", got "${line}"`);
        out.set(m[2], Number(m[1]));
    }
    return out;
}

const HEADER = `# Source files over ${LIMIT} lines, and the most lines each may have
# (scripts/check-file-sizes.mjs). A file here may only shrink: run
# \`node scripts/check-file-sizes.mjs --update\` after shrinking one. Adding a
# file or raising a number is a hand edit that the PR has to justify.
`;

export function formatBaseline(entries) {
    const sorted = [...entries].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
    return HEADER + sorted.map(([path, lines]) => `${lines} ${path}`).join("\n") + "\n";
}

// ── Compare ─────────────────────────────────────────────────────────────────

/**
 * `sizes` maps every source path to its line count; `baseline` maps listed
 * paths to their allowed count. Returns the four ways they can disagree.
 */
export function compare(sizes, baseline, limit = LIMIT) {
    const grew = [];
    const added = [];
    const shrank = [];
    const gone = [];
    for (const [path, allowed] of baseline) {
        const now = sizes.get(path);
        if (now === undefined) gone.push({ path, allowed });
        else if (now > allowed) grew.push({ path, now, allowed });
        else if (now < allowed) shrank.push({ path, now, allowed });
    }
    for (const [path, now] of sizes) {
        if (!baseline.has(path) && now > limit) added.push({ path, now });
    }
    const byPath = (a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0);
    return { grew: grew.sort(byPath), added: added.sort(byPath), shrank: shrank.sort(byPath), gone: gone.sort(byPath) };
}

/**
 * The baseline after `--update`: each number lowered to today's size, and a
 * file dropped once it is gone or back under the limit. Never raises a number
 * or adds a file.
 */
export function lowered(sizes, baseline, limit = LIMIT) {
    const out = new Map();
    for (const [path, allowed] of baseline) {
        const now = sizes.get(path);
        if (now === undefined || now <= limit) continue;
        out.set(path, Math.min(now, allowed));
    }
    return out;
}

// ── Measure ─────────────────────────────────────────────────────────────────

/** Line counts of every source file in the working tree. */
export function measure() {
    const r = spawnSync("git", ["ls-files", "-z", "--cached", "--others", "--exclude-standard"], {
        cwd: REPO_ROOT,
        maxBuffer: 1 << 28,
    });
    if (r.status !== 0) throw new Error(`git ls-files failed: ${r.stderr}`);
    const sizes = new Map();
    for (const path of new Set(r.stdout.toString().split("\0"))) {
        if (!path || !isSourcePath(path)) continue;
        const full = join(REPO_ROOT, path);
        if (!existsSync(full)) continue; // deleted in the working tree
        const buf = readFileSync(full);
        if (isGenerated(buf)) continue;
        sizes.set(path, countLines(buf));
    }
    return sizes;
}

// ── Main ────────────────────────────────────────────────────────────────────

const isMain = process.argv[1] && import.meta.url.endsWith(process.argv[1].replace(/\\/g, "/").split("/").pop());

if (isMain) {
    const sizes = measure();
    const baselinePath = join(REPO_ROOT, BASELINE_FILE);
    const baseline = parseBaseline(readFileSync(baselinePath, "utf8"));

    if (process.argv.includes("--update")) {
        const next = lowered(sizes, baseline);
        writeFileSync(baselinePath, formatBaseline(next));
        console.log(`check-file-sizes: wrote ${BASELINE_FILE} (${next.size} files).`);
        const { grew, added } = compare(sizes, next);
        if (grew.length || added.length) {
            console.log("Still over (--update never raises or adds; run the check for details).");
            process.exit(1);
        }
        process.exit(0);
    }

    const { grew, added, shrank, gone } = compare(sizes, baseline);
    if (grew.length) {
        console.error(`ERROR: these files grew past their number in ${BASELINE_FILE}:`);
        for (const f of grew)
            console.error(`  ${f.path}: ${f.now} lines, allowed ${f.allowed} (+${f.now - f.allowed})`);
        console.error("Put the new code in a new module, or move something out of the file to make room.");
        console.error(`If the growth can't be avoided, raise the number in ${BASELINE_FILE} and say why in the PR.`);
        console.error();
    }
    if (added.length) {
        console.error(`ERROR: these files are over the ${LIMIT}-line limit:`);
        for (const f of added) console.error(`  ${f.path}: ${f.now} lines`);
        console.error("Split the file along its own seams before it grows further.");
        console.error(`If you renamed or moved a file listed in ${BASELINE_FILE}, move its line to the new path.`);
        console.error();
    }
    if (shrank.length || gone.length) {
        console.error(`ERROR: ${BASELINE_FILE} is out of date. These files got smaller (thank you):`);
        for (const f of shrank) console.error(`  ${f.path}: ${f.now} lines, baseline ${f.allowed}`);
        for (const f of gone) console.error(`  ${f.path}: no longer a source file here (deleted, renamed or excluded)`);
        console.error(`Run \`node scripts/check-file-sizes.mjs --update\` and commit ${BASELINE_FILE},`);
        console.error("so the room they gave up can't be taken back.");
        console.error();
    }
    if (grew.length || added.length || shrank.length || gone.length) process.exit(1);

    console.log(
        `check-file-sizes: ok. ${baseline.size} files over ${LIMIT} lines, none grew; ${sizes.size} source files checked.`
    );
}
