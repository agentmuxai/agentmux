#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-import-ratchets.mjs — CI ratchets on how the layers import each other.
//
// The architecture audit of 2026-11-01 (finding 1) found that nothing stopped the
// layers from eroding, so `backend/` imports up into `server/`,
// the frontend has a large import cycle and its lower layers import the view. Each
// check records today's violations in a baseline file under scripts/import-ratchets/,
// fails a change that adds one, and lets the baseline only shrink (as
// check-file-sizes.mjs does for file length).
//
// THE THREE CHECKS
//
//   backend-server    srv's `backend/` must not name `crate::server`. The baseline
//                     holds `<count> <file>`: a file may not gain a reference, and a
//                     file not listed may not have one. Counted: production code
//                     only (test files, files declared under `#[cfg(test)]` as
//                     `mod name;`, and `#[cfg(test)]` items are skipped), comments
//                     and string literals blanked, every `crate::server` path (a `use`
//                     or a call) one reference.
//
//   frontend-cycles   a file may not join an import cycle. The baseline holds the
//                     files that are in one today. Only imports that survive
//                     compilation count: `import type`, `export type ... from` and
//                     `import { type A }` do not, nor does a dynamic `import()`. A
//                     cycle among files that are all listed already is not told apart
//                     from the one they are in, so the check holds the membership of
//                     the cycles, not their number.
//
//   frontend-inverted  a lower layer may not import a higher one (LAYER_RULES). The
//                     baseline holds the `importer -> imported` pairs. Type-only and
//                     dynamic imports count here: depending on a view's types is
//                     still depending on the view.
//
// WHAT IS SCANNED. Source files from `git ls-files` by the same rule as
// check-file-sizes.mjs (no tests, fixtures or generated files). Frontend: `.ts` and
// `.tsx` under frontend/. An import is found by a pattern on `import`/`export ... from`
// at the start of a line, in the code with comments and string contents blanked, not by a
// parser, so the script needs no install step;
// check-import-ratchets.test.mjs pins what it reads.
//
// Usage:
//   node scripts/check-import-ratchets.mjs            check all three
//   node scripts/check-import-ratchets.mjs --update   lower the baselines to today's
//                                                     state (drops fixed entries and
//                                                     lowers counts; never adds)
//   node scripts/check-import-ratchets.mjs --init     record today's state, adding
//                                                     entries: to create the baselines

import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { isMain } from "./lib/is-main.mjs";
import { isGenerated, isSourcePath } from "./check-file-sizes.mjs";
import { cfgTestModules, maskRust, maskTs, rustCode } from "./lib/source-mask.mjs";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const BASELINE_DIR = "scripts/import-ratchets";

/** The layers, lowest first: the importer may not reach into the target. */
export const LAYER_RULES = [
    { name: "store -> view", from: "frontend/app/store/", to: "frontend/app/view/" },
    { name: "element -> view", from: "frontend/app/element/", to: "frontend/app/view/" },
    { name: "util -> app", from: "frontend/util/", to: "frontend/app/" },
];

// ── Rust: backend/ naming server/ ───────────────────────────────────────────

const RUST_BACKEND = "crates/srv/src/backend/";
const SERVER_PATH = /\bcrate::server\b/g;

/** `crate::server` paths in one Rust file's production code. */
export function serverRefs(src) {
    return (rustCode(src).match(SERVER_PATH) ?? []).length;
}

// ── TypeScript: imports ─────────────────────────────────────────────────────

const FROM_STATEMENT = /^[ \t]*(import|export)\b([^;'"`]*?)\bfrom\s*(["'])([^"'\n]*)\3/gmd;
const SIDE_EFFECT_IMPORT = /^[ \t]*import\s*(["'])([^"'\n]*)\1/gmd;
const DYNAMIC_IMPORT = /\bimport\(\s*(["'])([^"'\n]*)\1\s*\)/gd;

/** Whether the clause between `import`/`export` and `from` brings in types only. */
export function isTypeOnlyClause(clause) {
    const c = clause.trim();
    if (/^type[\s{*]/.test(c)) return true;
    const named = /^\{([\s\S]*)\}$/.exec(c);
    if (!named) return false;
    const names = named[1].split(",").map((s) => s.trim()).filter(Boolean);
    return names.length > 0 && names.every((n) => /^type\s/.test(n));
}

/**
 * The module specifiers one TypeScript file imports:
 * `{ value: [...], type: [...], dynamic: [...] }`, each once.
 */
export function readImports(src) {
    // Match on code only (comments and the inside of strings blanked), then read the specifier from
    // the source at the same place.
    const code = maskTs(src);
    // The specifiers are read from `src` at the offsets found in `code`: they must line up.
    if (code.length !== src.length) throw new Error("maskTs changed the length of the source");
    const text = (m, group) => src.slice(m.indices[group][0], m.indices[group][1]);
    const value = new Set();
    const type = new Set();
    const dynamic = new Set();
    for (const m of code.matchAll(FROM_STATEMENT)) (isTypeOnlyClause(m[2]) ? type : value).add(text(m, 4));
    for (const m of code.matchAll(SIDE_EFFECT_IMPORT)) value.add(text(m, 2));
    for (const m of code.matchAll(DYNAMIC_IMPORT)) dynamic.add(text(m, 2));
    return { value: [...value], type: [...type], dynamic: [...dynamic] };
}

/** tsconfig `paths` as `[prefix, target]` pairs (`@/app/*` -> `@/app/`) and exact entries. */
export function aliasesOf(tsconfigText) {
    const paths = JSON.parse(tsconfigText).compilerOptions?.paths ?? {};
    const prefixes = [];
    const exact = new Map();
    for (const [pattern, targets] of Object.entries(paths)) {
        const target = String(targets[0]).replace(/^\.\//, "");
        if (pattern.endsWith("/*")) prefixes.push([pattern.slice(0, -1), target.replace(/\*$/, "")]);
        else exact.set(pattern, target);
    }
    return { prefixes, exact };
}

function normalize(path) {
    const out = [];
    for (const part of path.split("/")) {
        if (part === "." || part === "") continue;
        if (part === "..") out.pop();
        else out.push(part);
    }
    return out.join("/");
}

/** The repo file a specifier names, or null (a package, or a file that is not scanned). */
export function resolveSpecifier(from, spec, files, aliases) {
    let base;
    if (aliases.exact.has(spec)) base = aliases.exact.get(spec);
    else {
        const hit = aliases.prefixes.find(([prefix]) => spec.startsWith(prefix));
        if (hit) base = hit[1] + spec.slice(hit[0].length);
        else if (spec.startsWith(".")) base = normalize(`${from.slice(0, from.lastIndexOf("/") + 1)}${spec}`);
        else return null;
    }
    base = normalize(base);
    for (const candidate of [base, `${base}.ts`, `${base}.tsx`, `${base}/index.ts`, `${base}/index.tsx`]) {
        if (files.has(candidate)) return candidate;
    }
    return null;
}

/**
 * The import graph: `{ runtime, all }`, each a Map from a file to the Set of files it
 * imports. `runtime` is the static imports that survive compilation; `all` adds
 * type-only and dynamic ones. `sources` maps each scanned path to its text.
 */
export function importGraph(sources, aliases) {
    const files = new Set(sources.keys());
    const runtime = new Map();
    const all = new Map();
    for (const [file, src] of sources) {
        const found = readImports(src);
        const edges = (specs) => {
            const out = new Set();
            for (const spec of specs) {
                const to = resolveSpecifier(file, spec, files, aliases);
                if (to && to !== file) out.add(to);
            }
            return out;
        };
        const runtimeEdges = edges(found.value);
        runtime.set(file, runtimeEdges);
        all.set(file, new Set([...runtimeEdges, ...edges(found.type), ...edges(found.dynamic)]));
    }
    return { runtime, all };
}

/** The files that are in an import cycle: members of a strongly connected group of two or more. */
export function filesInCycles(graph) {
    let counter = 0;
    const index = new Map();
    const low = new Map();
    const onStack = new Set();
    const stack = [];
    const inCycle = new Set();
    for (const root of graph.keys()) {
        if (index.has(root)) continue;
        const open = (node) => {
            index.set(node, counter);
            low.set(node, counter);
            counter++;
            stack.push(node);
            onStack.add(node);
            return { node, next: [...(graph.get(node) ?? [])], at: 0 };
        };
        const frames = [open(root)];
        while (frames.length) {
            const frame = frames[frames.length - 1];
            if (frame.at < frame.next.length) {
                const to = frame.next[frame.at++];
                if (!graph.has(to)) continue;
                if (!index.has(to)) frames.push(open(to));
                else if (onStack.has(to)) low.set(frame.node, Math.min(low.get(frame.node), index.get(to)));
                continue;
            }
            if (low.get(frame.node) === index.get(frame.node)) {
                const group = [];
                let member;
                do {
                    member = stack.pop();
                    onStack.delete(member);
                    group.push(member);
                } while (member !== frame.node);
                if (group.length > 1) for (const file of group) inCycle.add(file);
            }
            frames.pop();
            if (frames.length) {
                const parent = frames[frames.length - 1].node;
                low.set(parent, Math.min(low.get(parent), low.get(frame.node)));
            }
        }
    }
    return inCycle;
}

/** The `importer -> imported` pairs that break a layer rule. */
export function invertedImports(graph, rules = LAYER_RULES) {
    const out = new Set();
    for (const [from, tos] of graph) {
        for (const to of tos) {
            if (rules.some((r) => from.startsWith(r.from) && to.startsWith(r.to))) out.add(`${from} -> ${to}`);
        }
    }
    return out;
}

// ── Baselines and comparison ────────────────────────────────────────────────

const byText = (a, b) => (a < b ? -1 : a > b ? 1 : 0);

/** One entry per line; `#` comments and blank lines ignored. */
export function parseLines(text) {
    return text.split("\n").map((l) => l.trim()).filter((l) => l && !l.startsWith("#"));
}

/** `<count> <path>` per line, as a Map. */
export function parseCounts(text, name = "baseline") {
    const out = new Map();
    for (const line of parseLines(text)) {
        const m = /^(\d+)\s+(\S+)$/.exec(line);
        if (!m) throw new Error(`${name}: expected "<count> <path>", got "${line}"`);
        out.set(m[2], Number(m[1]));
    }
    return out;
}

/** Entries of `now` that `baseline` does not allow, and entries of `baseline` that `now` has outgrown. */
export function compareSets(now, baseline) {
    return {
        added: [...now].filter((x) => !baseline.has(x)).sort(byText),
        fixed: [...baseline].filter((x) => !now.has(x)).sort(byText),
    };
}

/** Like [`compareSets`] for per-file counts: more than allowed is `grew`, less is `shrank`. */
export function compareCounts(now, baseline) {
    const grew = [];
    const shrank = [];
    for (const [path, count] of now) {
        const allowed = baseline.get(path) ?? 0;
        if (count > allowed) grew.push({ path, now: count, allowed });
    }
    for (const [path, allowed] of baseline) {
        const count = now.get(path) ?? 0;
        if (count < allowed) shrank.push({ path, now: count, allowed });
    }
    const order = (a, b) => byText(a.path, b.path);
    return { grew: grew.sort(order), shrank: shrank.sort(order) };
}

/** The baseline after `--update`: counts lowered, fixed entries dropped, nothing added or raised. */
export function loweredCounts(now, baseline) {
    const out = new Map();
    for (const [path, allowed] of baseline) {
        const count = now.get(path) ?? 0;
        if (count > 0) out.set(path, Math.min(count, allowed));
    }
    return out;
}

export function loweredSet(now, baseline) {
    return new Set([...baseline].filter((x) => now.has(x)));
}

function header(title, how) {
    return `# ${title}\n# (scripts/check-import-ratchets.mjs). ${how}\n# Run \`node scripts/check-import-ratchets.mjs --update\` after fixing some; adding\n# an entry is a hand edit that the PR has to justify.\n`;
}

export function formatCounts(counts) {
    const rows = [...counts].sort(([a], [b]) => byText(a, b)).map(([path, n]) => `${n} ${path}`);
    return header("Files under crates/srv/src/backend/ that name crate::server, and how many times", "A file may not name it more often than this, and a file not listed may not name it at all.") + rows.join("\n") + "\n";
}

export function formatSet(entries, title, how) {
    return header(title, how) + [...entries].sort(byText).join("\n") + "\n";
}

// ── Measure ─────────────────────────────────────────────────────────────────

function trackedFiles() {
    const r = spawnSync("git", ["ls-files", "-z", "--cached", "--others", "--exclude-standard"], { cwd: REPO_ROOT, maxBuffer: 1 << 28 });
    if (r.status !== 0) throw new Error(`git ls-files failed: ${r.stderr}`);
    return [...new Set(r.stdout.toString().split("\0"))].filter((p) => p && existsSync(join(REPO_ROOT, p)));
}

function readSource(path) {
    const buf = readFileSync(join(REPO_ROOT, path));
    return isGenerated(buf) ? null : buf.toString("utf8");
}

export function measure() {
    const paths = trackedFiles().filter(isSourcePath);
    const rustSources = new Map();
    const testOnly = new Set();
    const sources = new Map();
    for (const path of paths) {
        if (path.startsWith(RUST_BACKEND) && path.endsWith(".rs")) {
            const text = readSource(path);
            if (text === null) continue;
            rustSources.set(path, text);
            for (const t of cfgTestModules(path, maskRust(text))) testOnly.add(t);
        } else if (path.startsWith("frontend/") && /\.tsx?$/.test(path)) {
            const text = readSource(path);
            if (text !== null) sources.set(path, text);
        }
    }
    const rust = new Map();
    for (const [path, text] of rustSources) {
        // Test code however it is named, and everything under a folder declared that way.
        if (testOnly.has(path) || [...testOnly].some((t) => t.endsWith("/mod.rs") && path.startsWith(t.slice(0, -"mod.rs".length)))) continue;
        const n = serverRefs(text);
        if (n > 0) rust.set(path, n);
    }
    const aliases = aliasesOf(readFileSync(join(REPO_ROOT, "tsconfig.json"), "utf8"));
    const graph = importGraph(sources, aliases);
    return { backendServer: rust, cycles: filesInCycles(graph.runtime), inverted: invertedImports(graph.all), files: sources.size };
}

// ── Main ────────────────────────────────────────────────────────────────────

const BASELINES = {
    backendServer: `${BASELINE_DIR}/backend-server.baseline`,
    cycles: `${BASELINE_DIR}/frontend-cycles.baseline`,
    inverted: `${BASELINE_DIR}/frontend-inverted.baseline`,
};

function readBaseline(key) {
    return readFileSync(join(REPO_ROOT, BASELINES[key]), "utf8");
}

function writeBaselines(next) {
    mkdirSync(join(REPO_ROOT, BASELINE_DIR), { recursive: true });
    writeFileSync(join(REPO_ROOT, BASELINES.backendServer), formatCounts(next.backendServer));
    writeFileSync(
        join(REPO_ROOT, BASELINES.cycles),
        formatSet(next.cycles, "Frontend files that are in an import cycle", "A file not listed may not join a cycle. Type-only and dynamic imports do not count.")
    );
    writeFileSync(
        join(REPO_ROOT, BASELINES.inverted),
        formatSet(next.inverted, "Frontend imports from a lower layer into a higher one (LAYER_RULES in the script)", "An import not listed may not be added.")
    );
    console.log(`check-import-ratchets: wrote the baselines (${next.backendServer.size} backend files, ${next.cycles.size} cycle files, ${next.inverted.size} inverted imports).`);
}

if (isMain(import.meta.url)) {
    const now = measure();
    if (process.argv.includes("--init")) {
        // Records today's state as the baseline, adding entries: for creating the files, not for
        // making a failing check pass (use --update to shrink, a hand edit to allow).
        writeBaselines(now);
        process.exit(0);
    }
    const baseline = {
        backendServer: parseCounts(readBaseline("backendServer"), BASELINES.backendServer),
        cycles: new Set(parseLines(readBaseline("cycles"))),
        inverted: new Set(parseLines(readBaseline("inverted"))),
    };

    if (process.argv.includes("--update")) {
        const next = {
            backendServer: loweredCounts(now.backendServer, baseline.backendServer),
            cycles: loweredSet(now.cycles, baseline.cycles),
            inverted: loweredSet(now.inverted, baseline.inverted),
        };
        writeBaselines(next);
        process.exit(0);
    }

    let failed = false;
    const fail = (lines) => {
        failed = true;
        for (const l of lines) console.error(l);
        console.error();
    };

    const counts = compareCounts(now.backendServer, baseline.backendServer);
    if (counts.grew.length) {
        fail([
            "ERROR: backend/ names crate::server more than before. The server layer sits above backend/; move what both need down (a backend module, or agentmux_common), or pass it in:",
            ...counts.grew.map((f) => `  ${f.path}: ${f.now} references, allowed ${f.allowed}`),
        ]);
    }
    if (counts.shrank.length) {
        fail([
            `ERROR: ${BASELINES.backendServer} is out of date. These files name crate::server less (thank you):`,
            ...counts.shrank.map((f) => `  ${f.path}: ${f.now}, baseline ${f.allowed}`),
            "Run `node scripts/check-import-ratchets.mjs --update` and commit the baselines, so the room they gave up can't be taken back.",
        ]);
    }

    const cycles = compareSets(now.cycles, baseline.cycles);
    if (cycles.added.length) {
        fail([
            "ERROR: these frontend files are in an import cycle and were not before. Break the loop: import the value from the module that owns it, or `import type` where only a type is needed:",
            ...cycles.added.map((f) => `  ${f}`),
        ]);
    }
    if (cycles.fixed.length) {
        fail([
            `ERROR: ${BASELINES.cycles} is out of date. These files are out of every import cycle now (thank you):`,
            ...cycles.fixed.map((f) => `  ${f}`),
            "Run `node scripts/check-import-ratchets.mjs --update` and commit the baselines.",
        ]);
    }

    const inverted = compareSets(now.inverted, baseline.inverted);
    if (inverted.added.length) {
        fail([
            `ERROR: new imports from a lower layer into a higher one (${LAYER_RULES.map((r) => r.name).join(", ")}). Move the shared type or helper down to the lower layer instead:`,
            ...inverted.added.map((f) => `  ${f}`),
        ]);
    }
    if (inverted.fixed.length) {
        fail([
            `ERROR: ${BASELINES.inverted} is out of date. These imports are gone (thank you):`,
            ...inverted.fixed.map((f) => `  ${f}`),
            "Run `node scripts/check-import-ratchets.mjs --update` and commit the baselines.",
        ]);
    }
    if (failed) process.exit(1);

    console.log(
        `check-import-ratchets: ok. backend->server ${[...now.backendServer.values()].reduce((a, b) => a + b, 0)} references in ${now.backendServer.size} files; ${now.cycles.size} frontend files in cycles; ${now.inverted.size} inverted imports; ${now.files} frontend files read.`
    );
}
