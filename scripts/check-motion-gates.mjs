#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-motion-gates.mjs — CI gate: motion follows the one motion policy.
//
// docs/specs/SPEC_MOTION_TIERS_AND_STYLESHEET_DRY_2026_10_15.md §4.6
//
// THE RULE. frontend/app/motion/motion.ts is the only place that reads the OS
// motion preference. An animation declares its reduced treatment with
// `@include mixins.when-motion-reduced(<category>)` in SCSS, or asks
// `shouldReduceMotion(<category>)` in TS, and the policy decides whether it
// applies. A raw media query or a one-off matchMedia check follows the OS
// whatever the policy says: thirteen of them came back after the 2026-07-11
// decision that the app would not, because nothing stopped them.
//
// Fails, in frontend/ (every text file, tests and comments included), on:
//   1. `prefers-reduced-motion` outside ALLOWED_FILES;
//   2. a file that calls `matchMedia(` and mentions `reduced-motion`, outside
//      ALLOWED_FILES;
//   3. `@include ...when-motion-reduced(<x>)` where <x> isn't a category.
//      The mixin's own @error catches this at build time; this names the file.
// The vendored Font Awesome CSS has its own such rule, but it lives in
// public/fontawesome/, outside frontend/; app.scss's override handles it.
//
// A second part keeps hand-rolled motion from coming back, for the motion
// primitives (docs/specs/SPEC_MOTION_PRIMITIVES_2026_10_29.md §6). Code-driven
// motion goes through frontend/app/motion/primitives.ts, and a stylesheet's
// timings are tokens. Counted per file, outside comments and tests:
//   animate         `.animate(` or `startViewTransition(` in TS/JS outside
//                   frontend/app/motion/, util/view-transition.ts and EXCEPTIONS;
//   keyframes       `@keyframes` outside app/_keyframes.scss;
//   literal-ms      a literal time (`<n>ms` or `<n>s`) in an `animation` or
//                   `transition` declaration instead of a `var(--duration-*)`;
//                   0 and exactly 1ms (a reduced treatment's "at once") pass.
// scripts/check-motion-gates.baseline lists today's files and counts. A count may
// only go down, and a file whose count reached zero must leave the file:
// `node scripts/check-motion-gates.mjs --update` lowers the numbers after a
// migration. Adding a file or raising a number is a hand edit the PR must justify.
//
// Usage:
//   node scripts/check-motion-gates.mjs            check
//   node scripts/check-motion-gates.mjs --update   lower the baseline to today's counts
// Exit 0 = clean, exit 1 = a violation was found.

import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, extname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { isMain } from "./lib/is-main.mjs";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

export const CATEGORIES = ["status", "feedback", "transition", "decorative"];

export const ALLOWED_FILES = new Set([
    "frontend/app/mixins.scss", // the mixin's doc comment
    "frontend/app/motion/motion.ts", // the one query
    "frontend/app/motion/motion.test.ts", // its test
]);

const TEXT_EXT = new Set([".scss", ".css", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".html", ".svg"]);

const MATCH_MEDIA_CALL = /\bmatchMedia\s*(?:\?\.)?\s*\(/;
const INCLUDE = /@include\s+(?:[\w-]+\.)?when-motion-reduced\s*\(\s*([^)]*?)\s*\)/g;

const lineAt = (text, index) => text.slice(0, index).split("\n").length;

/**
 * Violations in one file: `{ path, line, rule, text }`, line 1-based.
 * `path` is repo-relative with forward slashes.
 */
export function checkFile(path, text) {
    const out = [];
    const allowed = ALLOWED_FILES.has(path);
    const lines = text.split("\n");
    if (!allowed) {
        lines.forEach((l, i) => {
            if (l.includes("prefers-reduced-motion")) out.push({ path, line: i + 1, rule: "media-query", text: l.trim() });
        });
        if (MATCH_MEDIA_CALL.test(text) && /reduced-motion/i.test(text)) {
            const i = lines.findIndex((l) => MATCH_MEDIA_CALL.test(l));
            out.push({ path, line: i + 1, rule: "match-media", text: lines[i].trim() });
        }
    }
    if (path.endsWith(".scss")) {
        for (const m of text.matchAll(INCLUDE)) {
            const category = m[1].replace(/^["']|["']$/g, "");
            if (!CATEGORIES.includes(category)) {
                out.push({ path, line: lineAt(text, m.index), rule: "category", text: m[0] });
            }
        }
    }
    return out;
}

function walk(dir, out = []) {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
        if (entry.name === "node_modules" || entry.name === "dist") continue;
        const full = join(dir, entry.name);
        if (entry.isDirectory()) walk(full, out);
        else if (TEXT_EXT.has(extname(entry.name))) out.push(full);
    }
    return out;
}

/** Every violation under `root`/frontend. */
export function collect(root = REPO_ROOT) {
    return walk(join(root, "frontend")).flatMap((full) =>
        checkFile(relative(root, full).split(sep).join("/"), readFileSync(full, "utf8"))
    );
}

// ── Hand-rolled motion ratchet ──────────────────────────────────────────────

const BASELINE_FILE = "scripts/check-motion-gates.baseline";

/** Where code-driven motion may be written: the primitives and the view-transition entry point. */
export const MOTION_HOMES = ["frontend/app/motion/", "frontend/app/util/view-transition.ts"];
/** Allowed to call `animate` itself, and why. */
export const EXCEPTIONS = new Map([
    // Schedules keyframes against the audio clock (startTime pinned to the sound's time), which is not a general shape.
    ["frontend/app/notification/activity-flash.ts", "pinned to the audio clock"],
]);
export const KEYFRAMES_HOME = "frontend/app/_keyframes.scss";

const SCRIPT_EXT = new Set([".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs"]);
const STYLE_EXT = new Set([".scss", ".css"]);
const isTest = (path) => /\.test\.[cm]?[jt]sx?$/.test(path) || path.includes("/__tests__/");

/** `text` with comments blanked out (newlines kept, so line numbers hold). */
function blankComments(text) {
    const blank = (m) => m.replace(/[^\n]/g, " ");
    // `//` starts a line comment unless a `:` precedes it (a URL).
    return text.replace(/\/\*[\s\S]*?\*\//g, blank).replace(/(^|[^:])(\/\/[^\n]*)/g, (_, lead, comment) => lead + blank(comment));
}

const ANIMATE_CALL = /\.animate\s*\(|\bstartViewTransition\s*\(/g;
const KEYFRAMES = /@keyframes\b/g;
const TIMING_DECL = /(?:^|[\s;{])((?:animation|transition)(?:-duration|-delay)?)\s*:\s*([^;{}]*)/g;
// A literal time in `ms` or `s`. Zero (no time) and exactly 1ms (a reduced treatment's "at
// once" that still fires the end events) are not durations to scale, so they pass.
const LITERAL_TIME = /(?<![\w.-])(\d*\.?\d+)(ms|s)\b/g;
const isScalableTime = ([, n, unit]) => Number(n) !== 0 && !(unit === "ms" && Number(n) === 1);
const hasLiteralTime = (value) => [...value.matchAll(LITERAL_TIME)].some(isScalableTime);

/**
 * The hand-rolled motion in one file: `{ rule, count, lines }` per rule that has any,
 * `lines` the 1-based line of each. `path` is repo-relative with forward slashes.
 */
export function countMotion(path, text) {
    if (isTest(path)) return [];
    const out = [];
    const ext = extname(path);
    const add = (rule, lines) => lines.length && out.push({ rule, count: lines.length, lines });
    if (SCRIPT_EXT.has(ext)) {
        const exempt = MOTION_HOMES.some((h) => (h.endsWith("/") ? path.startsWith(h) : path === h)) || EXCEPTIONS.has(path);
        if (!exempt) {
            const code = blankComments(text);
            add("animate", [...code.matchAll(ANIMATE_CALL)].map((m) => lineAt(code, m.index)));
        }
    } else if (STYLE_EXT.has(ext)) {
        const code = blankComments(text);
        if (path !== KEYFRAMES_HOME) add("keyframes", [...code.matchAll(KEYFRAMES)].map((m) => lineAt(code, m.index)));
        add(
            "literal-ms",
            [...code.matchAll(TIMING_DECL)].filter((m) => hasLiteralTime(m[2])).map((m) => lineAt(code, m.index + m[0].indexOf(m[1])))
        );
    }
    return out;
}

/** `rule path` -> count, for every file under `root`/frontend. */
export function collectCounts(root = REPO_ROOT) {
    const counts = new Map();
    for (const full of walk(join(root, "frontend"))) {
        const path = relative(root, full).split(sep).join("/");
        for (const { rule, count } of countMotion(path, readFileSync(full, "utf8"))) counts.set(`${rule} ${path}`, count);
    }
    return counts;
}

/** `<rule> <path> <count>` per line; `#` comments and blank lines ignored. */
export function parseBaseline(text) {
    const out = new Map();
    for (const [i, raw] of text.split("\n").entries()) {
        const line = raw.trim();
        if (!line || line.startsWith("#")) continue;
        const m = /^(animate|keyframes|literal-ms)\s+(\S+)\s+(\d+)$/.exec(line);
        if (!m) throw new Error(`${BASELINE_FILE}:${i + 1}: expected "<rule> <path> <count>", got "${line}"`);
        out.set(`${m[1]} ${m[2]}`, Number(m[3]));
    }
    return out;
}

const HEADER = `# Hand-rolled motion that predates the motion primitives, and the most each file
# may have (scripts/check-motion-gates.mjs; docs/specs/SPEC_MOTION_PRIMITIVES_2026_10_29.md §6).
# A count may only go down: migrate the file, then run
# \`node scripts/check-motion-gates.mjs --update\`. Adding a file or raising a number
# is a hand edit that the PR has to justify.
# <rule> <path> <count>
`;

export function formatBaseline(entries) {
    const sorted = [...entries].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
    return HEADER + sorted.map(([key, n]) => `${key} ${n}`).join("\n") + "\n";
}

/** How `counts` and `baseline` (both `rule path` -> count) disagree. */
export function compareBaseline(counts, baseline) {
    const grew = [];
    const added = [];
    const shrank = [];
    for (const [key, now] of counts) {
        const allowed = baseline.get(key);
        if (allowed === undefined) added.push({ key, now });
        else if (now > allowed) grew.push({ key, now, allowed });
    }
    for (const [key, allowed] of baseline) {
        const now = counts.get(key) ?? 0;
        if (now < allowed) shrank.push({ key, now, allowed });
    }
    return { grew, added, shrank };
}

/** The baseline after `--update`: each number lowered to today's count, a file at zero dropped, nothing added or raised. */
export function lowered(counts, baseline) {
    const next = new Map();
    for (const [key, allowed] of baseline) {
        const now = Math.min(counts.get(key) ?? 0, allowed);
        if (now > 0) next.set(key, now);
    }
    return next;
}

const RATCHET_HELP = {
    animate:
        "code-driven motion goes through frontend/app/motion/primitives.ts (play, move, travel, mergeInto, enter, exit, pulse, glide); " +
        "only the primitives call `animate`.",
    keyframes: "a new @keyframes belongs in frontend/app/_keyframes.scss, written once per shape.",
    "literal-ms": "use `var(--duration-*)` (theme.scss) instead of a literal duration.",
};

const HELP = {
    "media-query":
        "a raw prefers-reduced-motion query follows the OS whatever the motion policy says. " +
        "Declare the reduced treatment with `@include mixins.when-motion-reduced(<category>)`, " +
        "or ask `shouldReduceMotion(<category>)` from frontend/app/motion/motion.ts.",
    "match-media": "only frontend/app/motion/motion.ts reads the OS motion preference; use `shouldReduceMotion(<category>)`.",
    category: `the category must be one of: ${CATEGORIES.join(", ")}.`,
};

/** True when no hand-rolled motion grew; prints what did otherwise. */
function checkRatchet() {
    const counts = collectCounts();
    const baselinePath = join(REPO_ROOT, BASELINE_FILE);
    const baseline = parseBaseline(readFileSync(baselinePath, "utf8"));
    if (process.argv.includes("--update")) {
        writeFileSync(baselinePath, formatBaseline(lowered(counts, baseline)));
        console.log(`Lowered ${BASELINE_FILE} to today's counts.`);
        return true;
    }
    const { grew, added, shrank } = compareBaseline(counts, baseline);
    if (grew.length + added.length + shrank.length === 0) return true;
    console.error("ERROR: hand-rolled motion (docs/specs/SPEC_MOTION_PRIMITIVES_2026_10_29.md §6)");
    const over = [...added.map((v) => ({ ...v, why: "not in the baseline" })), ...grew.map((v) => ({ ...v, why: `baseline ${v.allowed}` }))];
    for (const rule of Object.keys(RATCHET_HELP)) {
        const hits = over.filter((v) => v.key.startsWith(`${rule} `));
        if (hits.length === 0) continue;
        console.error(`\n${rule}: ${RATCHET_HELP[rule]}`);
        for (const v of hits) console.error(`  ${v.key.slice(rule.length + 1)}: ${v.now} (${v.why})`);
    }
    if (shrank.length) {
        console.error("\nThese are below their baseline: run `node scripts/check-motion-gates.mjs --update` and commit it.");
        for (const v of shrank) console.error(`  ${v.key}: ${v.now}, baseline ${v.allowed}`);
    }
    return false;
}

function main() {
    const found = collect();
    if (found.length === 0) {
        if (!checkRatchet()) process.exit(1);
        console.log("OK: every motion gate in frontend/ goes through the motion policy; no new hand-rolled motion.");
        return;
    }
    console.error("ERROR: motion gated outside the motion policy");
    console.error("(docs/specs/SPEC_MOTION_TIERS_AND_STYLESHEET_DRY_2026_10_15.md §4.6):");
    for (const rule of Object.keys(HELP)) {
        const hits = found.filter((v) => v.rule === rule);
        if (hits.length === 0) continue;
        console.error(`\n${rule}: ${HELP[rule]}`);
        for (const v of hits) console.error(`  ${v.path}:${v.line}: ${v.text}`);
    }
    process.exit(1);
}

if (isMain(import.meta.url)) main();
