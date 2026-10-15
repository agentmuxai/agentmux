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
// Usage:
//   node scripts/check-motion-gates.mjs
// Exit 0 = clean, exit 1 = a violation was found.

import { readdirSync, readFileSync } from "node:fs";
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

const HELP = {
    "media-query":
        "a raw prefers-reduced-motion query follows the OS whatever the motion policy says. " +
        "Declare the reduced treatment with `@include mixins.when-motion-reduced(<category>)`, " +
        "or ask `shouldReduceMotion(<category>)` from frontend/app/motion/motion.ts.",
    "match-media": "only frontend/app/motion/motion.ts reads the OS motion preference; use `shouldReduceMotion(<category>)`.",
    category: `the category must be one of: ${CATEGORIES.join(", ")}.`,
};

function main() {
    const found = collect();
    if (found.length === 0) {
        console.log("OK: every motion gate in frontend/ goes through the motion policy.");
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
