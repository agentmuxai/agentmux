#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-function-sizes.mjs — CI ratchet on the length of functions.
//
// The architecture audit of 2026-11-01 (finding 2) counted about 700 functions over
// 100 lines across the TypeScript and Rust code, the longest over a thousand. Splitting
// them is a long job; this stops the count growing while it is done, as
// check-file-sizes.mjs does for files.
//
// THE RULES. A function may have at most LIMIT lines, unless it is listed in
// scripts/function-sizes.baseline, which holds every function that was already over.
// For a listed function:
//
//   - it may not grow past its baseline number;
//   - when it shrinks, or is removed, its line must come down with it, so the room it
//     gave up can't be taken back. `--update` does that.
//
// A function that is not listed may not pass the limit. Raising a number, or adding an
// entry, is a hand edit of the baseline that the PR has to explain.
//
// THE NAME OF A FUNCTION is `<path>::<name>`, so the baseline holds no line numbers and
// does not move when code above a function does. A method is `<path>::<Type>::<name>`
// (the `impl` or `trait` type in Rust, the class in TypeScript). Two listed functions
// with the same name in one file are told apart by order: the second is `<name>#2`.
// Renaming a listed function, or moving it to another file, is a hand edit of its line.
//
// WHAT COUNTS. `.rs`, `.ts` and `.tsx` source files by the same rule as
// check-file-sizes.mjs (no tests, fixtures or generated files). In Rust, production code
// only: `#[cfg(test)]` items and the files they declare are skipped. A function's length
// is the lines from the one with `fn` or `function` (or the `const` that names an arrow
// function) to the one with its closing bracket. Nested functions are counted on their
// own and also inside the one around them.
//   Rust:        every `fn` that has a body.
//   TypeScript:  `function` declarations and expressions that have a name; `const`/`let`/
//                `var` bound to a function or an arrow function (also inside a call such
//                as `memo(...)` or `useCallback(...)`), whose body is `{ ... }` or
//                `( ... )`; class methods; and object or class properties bound to a
//                function. An unnamed callback is not counted on its own.
// The functions are found by reading brackets in the code with comments and the inside of
// strings blanked (lib/source-mask.mjs), not by a parser, so the script needs no install
// step; check-function-sizes.test.mjs pins what it reads.
//
// Usage:
//   node scripts/check-function-sizes.mjs            check
//   node scripts/check-function-sizes.mjs --update   lower the baseline to today's
//                                                    lengths (never raises or adds), for
//                                                    the files this branch changed since
//                                                    origin/main
//   node scripts/check-function-sizes.mjs --update --all   the same, for every file
//   node scripts/check-function-sizes.mjs --init     record today's state, adding entries:
//                                                    to create the baseline

import { spawnSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { isMain } from "./lib/is-main.mjs";
import { changedSinceMain, compare, isGenerated, isSourcePath, lowered } from "./check-file-sizes.mjs";
import { maskTs, productionRustPaths, rustCode } from "./lib/source-mask.mjs";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const BASELINE_FILE = "scripts/function-sizes.baseline";

export const LIMIT = 100;

// ── Reading brackets ────────────────────────────────────────────────────────

const WS = /\s/;

function skipWs(m, i) {
    while (i < m.length && WS.test(m[i])) i++;
    return i;
}

/** The index of the bracket closing the one at `m[open]` (`(`, `[` or `{`), or -1. */
export function closeOf(m, open) {
    const o = m[open];
    const c = o === "(" ? ")" : o === "[" ? "]" : "}";
    let depth = 0;
    for (let j = open; j < m.length; j++) {
        if (m[j] === o) depth++;
        else if (m[j] === c && --depth === 0) return j;
    }
    return -1;
}

/** The index just past the `<...>` (type arguments) that starts at `m[i]`, or -1. */
function skipAngles(m, i) {
    let depth = 0;
    for (let j = i; j < m.length; j++) {
        const ch = m[j];
        if (ch === "<") depth++;
        else if (ch === ">" && m[j - 1] !== "=") {
            if (--depth === 0) return j + 1;
        } else if (ch === "(" || ch === "[" || ch === "{") {
            j = closeOf(m, j);
            if (j < 0) return -1;
        } else if (ch === ";") return -1;
    }
    return -1;
}

/** 1-based line of an offset in `m`. */
function lineIndex(m) {
    const starts = [0];
    for (let i = 0; i < m.length; i++) if (m[i] === "\n") starts.push(i + 1);
    return (idx) => {
        let lo = 0;
        let hi = starts.length - 1;
        while (lo < hi) {
            const mid = (lo + hi + 1) >> 1;
            if (starts[mid] <= idx) lo = mid;
            else hi = mid - 1;
        }
        return lo + 1;
    };
}

// ── Rust ────────────────────────────────────────────────────────────────────

const RUST_FN = /\bfn\s+([A-Za-z_][A-Za-z0-9_]*)/g;
const RUST_CONTAINER = /\b(impl|trait)\b/g;

/** The index of the `{` that opens a Rust function's body, or -1 for a declaration with none. */
function rustBody(m, from) {
    let depth = 0;
    for (let i = from; i < m.length; i++) {
        const ch = m[i];
        if (ch === "(" || ch === "[") depth++;
        else if (ch === ")" || ch === "]") {
            if (--depth < 0) return -1;
        } else if (depth === 0 && ch === ";") return -1;
        else if (depth === 0 && ch === "{") return i;
    }
    return -1;
}

/** The type an `impl` or `trait` head names (`impl<T> A for B<T> where ...` is `B`), or null. */
export function containerName(head) {
    let h = head.trim();
    if (h.startsWith("<")) {
        const end = skipAngles(h, 0);
        if (end < 0) return null;
        h = h.slice(end);
    }
    h = h.split(/\bwhere\b/)[0];
    const parts = h.split(/\bfor\b/);
    h = parts[parts.length - 1];
    for (let prev = null; prev !== h; ) {
        prev = h;
        h = h.replace(/<[^<>]*>/g, "");
    }
    // A trait head may carry bounds (`Visitor: Send + Sync`); a path's `::` is not one.
    const m = /([A-Za-z_][A-Za-z0-9_]*)[^A-Za-z0-9_]*$/.exec(h.trim().replace(/(?<!:):(?!:).*$/, "").replace(/\+.*$/, ""));
    return m ? m[1] : null;
}

/**
 * The `impl` and `trait` blocks of Rust code, as `{ name, open, close }`. Only where the
 * keyword starts an item, so `impl Trait` in an argument or return type is not one.
 */
function rustContainers(m) {
    const out = [];
    for (const k of m.matchAll(RUST_CONTAINER)) {
        const before = m.slice(Math.max(0, k.index - 40), k.index).trimEnd();
        if (before !== "" && !/(?:[};{\]]|\bpub|\bunsafe|\bdefault|\bauto|\))$/.test(before)) continue;
        const open = rustBody(m, k.index + k[0].length);
        if (open < 0) continue;
        const close = closeOf(m, open);
        const name = containerName(m.slice(k.index + k[0].length, open));
        if (close > 0 && name) out.push({ name, open, close });
    }
    return out;
}

function innermost(containers, idx) {
    let best = null;
    for (const c of containers) if (c.open < idx && idx < c.close && (!best || c.open > best.open)) best = c;
    return best;
}

/** Every Rust function with a body in `masked` (see [`rustCode`]), as `{ name, start, end }` (offsets). */
export function rustFunctions(masked) {
    const containers = rustContainers(masked);
    const out = [];
    for (const f of masked.matchAll(RUST_FN)) {
        const open = rustBody(masked, f.index + f[0].length);
        if (open < 0) continue;
        const close = closeOf(masked, open);
        if (close < 0) continue;
        const owner = innermost(containers, f.index);
        out.push({ name: owner ? `${owner.name}::${f[1]}` : f[1], start: f.index, end: close });
    }
    return out;
}

// ── TypeScript ──────────────────────────────────────────────────────────────

const IDENT = /[A-Za-z_$][\w$]*/y;
const KEYWORDS = new Set([
    "if", "for", "while", "switch", "catch", "return", "case", "default", "function", "with", "else",
    "typeof", "new", "await", "yield", "do", "try", "finally", "throw", "delete", "void", "in", "of",
    "instanceof", "super", "import", "export", "const", "let", "var", "class", "extends",
]);

/**
 * The index of the `{` opening a function body whose parameters end just before `i`,
 * or -1 (a signature without a body, or not a function). A return type may sit between,
 * and a `{` right after `:`, `|`, `&`, `,`, `<` or `(` is a type, not the body.
 */
function bodyBrace(m, i) {
    let prev = ")";
    for (let j = i; j < m.length; j++) {
        const ch = m[j];
        if (WS.test(ch)) continue;
        if (ch === "{") {
            if (!/[:|&,<(]/.test(prev)) return j;
            j = closeOf(m, j);
            if (j < 0) return -1;
            prev = "}";
        } else if (ch === "(" || ch === "[") {
            j = closeOf(m, j);
            if (j < 0) return -1;
            prev = ch === "(" ? ")" : "]";
        } else if (ch === "<") {
            const end = skipAngles(m, j);
            if (end < 0) return -1;
            j = end - 1;
            prev = ">";
        } else if (ch === ";" || ch === "}" || ch === ")" || ch === "]") {
            return -1;
        } else if (ch === "=" && m[j + 1] !== ">") {
            return -1;
        } else prev = ch;
    }
    return -1;
}

/** The index of the first char of an arrow function's body (`{` or `(`) if one starts at `i`, else -1. */
function arrowBody(m, i) {
    i = skipWs(m, i);
    const asyncKw = /async\b(?!\s*=>)/y;
    asyncKw.lastIndex = i;
    if (asyncKw.test(m)) i = skipWs(m, asyncKw.lastIndex);
    let j = i;
    if (m[j] === "<") {
        j = skipAngles(m, j);
        if (j < 0) return -1;
        j = skipWs(m, j);
    }
    if (m[j] === "(") {
        j = closeOf(m, j);
        if (j < 0) return -1;
        j++;
    } else {
        IDENT.lastIndex = j;
        if (!IDENT.test(m)) return -1;
        j = IDENT.lastIndex;
    }
    j = skipWs(m, j);
    if (m[j] === ":") {
        // A return type, up to the `=>` outside any bracket.
        for (j++; j < m.length && !(m[j] === "=" && m[j + 1] === ">"); j++) {
            if (m[j] === "(" || m[j] === "[" || m[j] === "{") {
                j = closeOf(m, j);
                if (j < 0) return -1;
            } else if (m[j] === "<") {
                const end = skipAngles(m, j);
                if (end < 0) return -1;
                j = end - 1;
            } else if (m[j] === ";") return -1;
        }
    }
    if (!(m[j] === "=" && m[j + 1] === ">")) return -1;
    j = skipWs(m, j + 2);
    return m[j] === "{" || m[j] === "(" ? j : -1;
}

/**
 * The index of the bracket that closes the function expression starting at `m[i]`: a
 * `function`, an arrow function, or one of those passed to a call such as `memo(...)`;
 * -1 if the text is not one.
 */
function functionEnd(m, i, depth = 0) {
    i = skipWs(m, i);
    const fn = /(?:async\s+)?function\b\s*\*?\s*(?:[A-Za-z_$][\w$]*)?\s*/y;
    fn.lastIndex = i;
    if (fn.test(m)) {
        let j = fn.lastIndex;
        if (m[j] === "<") {
            j = skipAngles(m, j);
            if (j < 0) return -1;
            j = skipWs(m, j);
        }
        if (m[j] !== "(") return -1;
        const params = closeOf(m, j);
        if (params < 0) return -1;
        const body = bodyBrace(m, params + 1);
        return body < 0 ? -1 : closeOf(m, body);
    }
    const arrow = arrowBody(m, i);
    if (arrow >= 0) return closeOf(m, arrow);
    if (depth < 3) {
        const call = /[A-Za-z_$][\w$.]*/y;
        call.lastIndex = i;
        if (call.test(m)) {
            let j = skipWs(m, call.lastIndex);
            if (m[j] === "<") {
                j = skipAngles(m, j);
                if (j < 0) return -1;
                j = skipWs(m, j);
            }
            if (m[j] === "(") return functionEnd(m, j + 1, depth + 1);
        }
    }
    return -1;
}

/** From the char after a `:` annotation, the index of the `=` that starts the value, or -1. */
function skipAnnotation(m, i) {
    for (let j = i; j < m.length; j++) {
        const ch = m[j];
        if (ch === "(" || ch === "[" || ch === "{") {
            j = closeOf(m, j);
            if (j < 0) return -1;
        } else if (ch === "<") {
            const end = skipAngles(m, j);
            if (end < 0) return -1;
            j = end - 1;
        } else if (ch === "=" && m[j + 1] === ">") j++;
        else if (ch === "=") return j;
        else if (ch === ";") return -1;
    }
    return -1;
}

const TS_DECL = /\b(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*/g;
const TS_METHOD =
    /^[ \t]*(?:(?:public|private|protected|static|async|readonly|override|abstract|get|set)\s+)*\*?\s*([A-Za-z_$][\w$]*)\s*(?=[<(])/gm;
const TS_PROPERTY =
    /^[ \t]*(?:(?:public|private|protected|static|readonly|override)\s+)*([A-Za-z_$][\w$]*)\s*[:=]\s*(?![=>])/gm;
const TS_FUNCTION = /\bfunction\b\s*\*?\s*([A-Za-z_$][\w$]*)\s*(?=[<(])/g;
const TS_CLASS = /\bclass\s+([A-Za-z_$][\w$]*)/g;

function tsContainers(m) {
    const out = [];
    for (const k of m.matchAll(TS_CLASS)) {
        let open = -1;
        for (let j = k.index + k[0].length; j < m.length; j++) {
            const ch = m[j];
            if (ch === "{") {
                open = j;
                break;
            }
            if (ch === "(" || ch === "[") j = closeOf(m, j);
            else if (ch === "<") j = skipAngles(m, j) - 1;
            else if (ch === ";") break;
            if (j < 0) break;
        }
        const close = open < 0 ? -1 : closeOf(m, open);
        if (close > 0) out.push({ name: k[1], open, close });
    }
    return out;
}

/** The end of a method whose name was matched ending at `from`: `<T>(params): ret { body }`. */
function methodEnd(m, from) {
    let j = from;
    if (m[j] === "<") {
        j = skipAngles(m, j);
        if (j < 0) return -1;
        j = skipWs(m, j);
    }
    if (m[j] !== "(") return -1;
    const params = closeOf(m, j);
    if (params < 0) return -1;
    const body = bodyBrace(m, params + 1);
    return body < 0 ? -1 : closeOf(m, body);
}

/**
 * The named functions of TypeScript in `masked` (see [`maskTs`]), as `{ name, start, end }`
 * (offsets). Which forms are read is listed at the head of this file.
 */
export function tsFunctions(masked) {
    const containers = tsContainers(masked);
    const found = [];
    const add = (name, start, end) => {
        if (end > start) found.push({ name, start, end });
    };
    for (const d of masked.matchAll(TS_DECL)) {
        let j = d.index + d[0].length;
        if (masked[j] === ":") {
            j = skipAnnotation(masked, j + 1);
            if (j < 0) continue;
        }
        if (masked[j] !== "=" || masked[j + 1] === "=" || masked[j + 1] === ">") continue;
        add(d[1], d.index, functionEnd(masked, j + 1));
    }
    for (const p of masked.matchAll(TS_PROPERTY)) {
        if (KEYWORDS.has(p[1])) continue;
        add(p[1], p.index + p[0].search(/\S/), functionEnd(masked, p.index + p[0].length));
    }
    for (const mt of masked.matchAll(TS_METHOD)) {
        if (KEYWORDS.has(mt[1])) continue;
        add(mt[1], mt.index + mt[0].search(/\S/), methodEnd(masked, mt.index + mt[0].length));
    }
    for (const f of masked.matchAll(TS_FUNCTION)) {
        const params = skipWs(masked, f.index + f[0].length);
        const close = methodEnd(masked, params);
        add(f[1], f.index, close);
    }
    // The same function can be read by two forms (`const f = function g() {}`): keep the first.
    const byEnd = new Map();
    for (const f of found) if (!byEnd.has(f.end)) byEnd.set(f.end, f);
    return [...byEnd.values()].map((f) => {
        const owner = innermost(containers, f.start);
        return owner ? { ...f, name: `${owner.name}::${f.name}` } : f;
    });
}

// ── Lengths ─────────────────────────────────────────────────────────────────

/**
 * The functions of one file longer than `limit` lines, as `{ name, lines }` in order of
 * appearance; the second of two with the same name is `name#2`. `masked` is [`rustCode`]
 * or [`maskTs`] output, for `lang` "rust" or "ts".
 */
export function functionsOver(masked, lang, limit = LIMIT) {
    const fns = lang === "rust" ? rustFunctions(masked) : tsFunctions(masked);
    const lineOf = lineIndex(masked);
    const long = fns
        .map((f) => ({ name: f.name, start: f.start, lines: lineOf(f.end) - lineOf(f.start) + 1 }))
        .filter((f) => f.lines > limit)
        .sort((a, b) => a.start - b.start);
    const seen = new Map();
    return long.map((f) => {
        const n = (seen.get(f.name) ?? 0) + 1;
        seen.set(f.name, n);
        return { name: n === 1 ? f.name : `${f.name}#${n}`, lines: f.lines };
    });
}

// ── The baseline file ───────────────────────────────────────────────────────

const byText = (a, b) => (a < b ? -1 : a > b ? 1 : 0);

/** `<lines> <path>::<name>` per line; `#` comments and blank lines ignored. */
export function parseBaseline(text) {
    const out = new Map();
    for (const [i, raw] of text.split("\n").entries()) {
        const line = raw.trim();
        if (!line || line.startsWith("#")) continue;
        const m = /^(\d+)\s+(\S+::\S+)$/.exec(line);
        if (!m) throw new Error(`${BASELINE_FILE}:${i + 1}: expected "<lines> <path>::<name>", got "${line}"`);
        out.set(m[2], Number(m[1]));
    }
    return out;
}

const HEADER = `# Functions over ${LIMIT} lines, and the most lines each may have
# (scripts/check-function-sizes.mjs). A function here may only shrink: run
# \`node scripts/check-function-sizes.mjs --update\` after shrinking one. Adding an
# entry or raising a number is a hand edit that the PR has to justify.
# Format: <lines> <path>::<name>; a method is <path>::<Type>::<name>.
`;

export function formatBaseline(entries) {
    const sorted = [...entries].sort(([a], [b]) => byText(a, b));
    return HEADER + sorted.map(([key, lines]) => `${lines} ${key}`).join("\n") + "\n";
}

// ── Measure ─────────────────────────────────────────────────────────────────

/** Length of every function over the limit in the working tree, keyed by `<path>::<name>`. */
export function measure() {
    const r = spawnSync("git", ["ls-files", "-z", "--cached", "--others", "--exclude-standard"], {
        cwd: REPO_ROOT,
        maxBuffer: 1 << 28,
    });
    if (r.status !== 0) throw new Error(`git ls-files failed: ${r.stderr}`);
    const rust = new Map();
    const ts = new Map();
    for (const path of new Set(r.stdout.toString().split("\0"))) {
        if (!path || !isSourcePath(path) || !existsSync(join(REPO_ROOT, path))) continue;
        const target = path.endsWith(".rs") ? rust : /\.tsx?$/.test(path) ? ts : null;
        if (!target) continue;
        const buf = readFileSync(join(REPO_ROOT, path));
        if (!isGenerated(buf)) target.set(path, buf.toString("utf8"));
    }
    const sizes = new Map();
    const byLang = { rust: 0, ts: 0 };
    const take = (path, masked, lang) => {
        for (const f of functionsOver(masked, lang)) {
            sizes.set(`${path}::${f.name}`, f.lines);
            byLang[lang]++;
        }
    };
    for (const path of productionRustPaths(rust)) take(path, rustCode(rust.get(path)), "rust");
    for (const [path, text] of ts) take(path, maskTs(text), "ts");
    return { sizes, byLang, files: rust.size + ts.size };
}

// ── Main ────────────────────────────────────────────────────────────────────

if (isMain(import.meta.url)) {
    const { sizes, byLang, files } = measure();
    const baselinePath = join(REPO_ROOT, BASELINE_FILE);

    if (process.argv.includes("--init")) {
        // Records today's state, adding entries: for creating the baseline, not for making a
        // failing check pass (use --update to shrink, a hand edit to allow).
        writeFileSync(baselinePath, formatBaseline(sizes));
        console.log(`check-function-sizes: wrote ${BASELINE_FILE} (${sizes.size} functions).`);
        process.exit(0);
    }

    const baseline = parseBaseline(readFileSync(baselinePath, "utf8"));

    if (process.argv.includes("--update")) {
        const all = process.argv.includes("--all");
        const changed = all ? null : changedSinceMain();
        if (!changed && !all) console.log("check-function-sizes: no origin/main to compare with; lowering every entry.");
        const scope = changed
            ? new Set([...baseline.keys(), ...sizes.keys()].filter((key) => changed.has(key.split("::")[0])))
            : null;
        const next = lowered(sizes, baseline, LIMIT, scope);
        writeFileSync(baselinePath, formatBaseline(next));
        console.log(`check-function-sizes: wrote ${BASELINE_FILE} (${next.size} functions).`);
        const { grew, added } = compare(sizes, next, LIMIT);
        if (grew.length || added.length) {
            console.log("Still over (--update never raises or adds; run the check for details).");
            process.exit(1);
        }
        process.exit(0);
    }

    const { grew, added, shrank, gone } = compare(sizes, baseline, LIMIT);
    if (grew.length) {
        console.error(`ERROR: these functions grew past their number in ${BASELINE_FILE}:`);
        for (const f of grew) console.error(`  ${f.path}: ${f.now} lines, allowed ${f.allowed} (+${f.now - f.allowed})`);
        console.error("Put the new code in a function of its own, or move something out of this one to make room.");
        console.error(`If the growth can't be avoided, raise the number in ${BASELINE_FILE} and say why in the PR.`);
        console.error();
    }
    if (added.length) {
        console.error(`ERROR: these functions are over the ${LIMIT}-line limit:`);
        for (const f of added) console.error(`  ${f.path}: ${f.now} lines`);
        console.error("Split the function along its own steps: each one a function with a name that says what it does.");
        console.error(`If you renamed or moved a function listed in ${BASELINE_FILE}, move its line to the new name.`);
        console.error();
    }
    if (shrank.length || gone.length) {
        console.error(`ERROR: ${BASELINE_FILE} is out of date. These functions got shorter or are gone (thank you):`);
        for (const f of shrank) console.error(`  ${f.path}: ${f.now} lines, baseline ${f.allowed}`);
        for (const f of gone) console.error(`  ${f.path}: not over ${LIMIT} lines any more (shorter, removed, renamed or moved)`);
        console.error(`Run \`node scripts/check-function-sizes.mjs --update\` and commit ${BASELINE_FILE},`);
        console.error("so the room they gave up can't be taken back.");
        console.error();
    }
    if (grew.length || added.length || shrank.length || gone.length) process.exit(1);

    console.log(
        `check-function-sizes: ok. ${baseline.size} functions over ${LIMIT} lines, none grew; ${files} source files read (${byLang.rust} Rust and ${byLang.ts} TypeScript over the limit).`
    );
}
