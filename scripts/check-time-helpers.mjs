#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-time-helpers.mjs — CI ratchet for hand-rolled "current time" code.
//
// `agentmux_common::time` (now_ms, now_ms_u64, now_secs, now_secs_u64) is the
// one home for "milliseconds/seconds since the Unix epoch". It was lifted in
// #3033, but nothing pointed new code at it, and copies regrew
// (docs/specs/SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §6.1).
//
// Fails when a Rust file outside agentmux-common, migrations and tests/
// either computes it inline (`SystemTime::now()...duration_since(UNIX_EPOCH)`)
// or defines its own `fn now_ms` / `now_secs` / `now_unix_secs` / `now_millis`,
// unless the file is on the grandfather list below. Also fails when a listed
// file no longer matches, so the list only shrinks: when you migrate a file to
// `agentmux_common::time`, delete its line here.
//
//   node scripts/check-time-helpers.mjs            check
//   node scripts/check-time-helpers.mjs --list     print today's matching files
//
// Migrations are exempt on purpose: they freeze copies of live logic
// (crates/srv/src/migrations/mod.rs).

import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative, sep } from "node:path";

const ROOT = new URL("..", import.meta.url).pathname;
const CRATES = ["crates/srv", "crates/mcp", "crates/launcher", "crates/bashwrap", "crates/cef"];
const INLINE = /SystemTime::now\(\)\s*\.duration_since\(\s*(?:std::time::)?UNIX_EPOCH/;
const OWN_FN = /\bfn\s+(?:now_ms|now_secs|now_unix_secs|now_millis)\b/;
const LIST_FILE = join(ROOT, "scripts/check-time-helpers.allow");

function* rustFiles(dir) {
    for (const name of readdirSync(dir)) {
        const p = join(dir, name);
        const st = statSync(p);
        if (st.isDirectory()) {
            if (name === "target" || name === "node_modules" || name === "migrations" || name === "tests") continue;
            yield* rustFiles(p);
        } else if (name.endsWith(".rs")) {
            yield p;
        }
    }
}

const matching = [];
for (const crate of CRATES) {
    let st;
    try {
        st = statSync(join(ROOT, crate));
    } catch {
        continue;
    }
    if (!st.isDirectory()) continue;
    for (const file of rustFiles(join(ROOT, crate))) {
        const text = readFileSync(file, "utf8");
        if (INLINE.test(text) || OWN_FN.test(text)) matching.push(relative(ROOT, file).split(sep).join("/"));
    }
}
matching.sort();

if (process.argv.includes("--list")) {
    console.log(matching.join("\n"));
    process.exit(0);
}

const allowed = new Set(
    readFileSync(LIST_FILE, "utf8")
        .split("\n")
        .map((l) => l.trim())
        .filter((l) => l && !l.startsWith("#")),
);
const added = matching.filter((f) => !allowed.has(f));
const stale = [...allowed].filter((f) => !matching.includes(f));

if (added.length || stale.length) {
    if (added.length) {
        console.error("check-time-helpers: new hand-rolled epoch-time code (use agentmux_common::time instead):");
        for (const f of added) console.error(`  ${f}`);
    }
    if (stale.length) {
        console.error("check-time-helpers: these files no longer hand-roll epoch time; delete their lines from scripts/check-time-helpers.allow:");
        for (const f of stale) console.error(`  ${f}`);
    }
    process.exit(1);
}
console.log(`check-time-helpers: ok (${allowed.size} grandfathered files)`);
