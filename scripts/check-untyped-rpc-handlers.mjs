#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-untyped-rpc-handlers.mjs — CI ratchet on untyped RPC handlers.
//
// `RpcEngine::register_typed` takes a typed request and response, records the
// command's schema, and (for types that derive ts-rs) is what
// check-rpc-bindings.sh keeps in sync with the frontend. `register_handler`
// takes raw JSON. The 2026-09-30 large-file analysis counted 49 untyped
// registrations; by 2026-10-09 there were 59
// (docs/specs/PLAN_CI_TEST_SPEED_AND_DRY_FOLLOWUPS_2026_10_09.md step 12).
//
// Fails when crates/srv registers more untyped handlers than MAX, outside
// tests. Also fails when it registers fewer, so the number only goes down:
// when you move a handler to register_typed, lower MAX to the new count.
//
//   node scripts/check-untyped-rpc-handlers.mjs          check
//   node scripts/check-untyped-rpc-handlers.mjs --list   print each one

import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

const MAX = 59;

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const SRC = join(ROOT, "crates/srv/src");

function* rustFiles(dir) {
    for (const name of readdirSync(dir)) {
        const p = join(dir, name);
        if (statSync(p).isDirectory()) {
            if (name !== "tests") yield* rustFiles(p);
        } else if (name.endsWith(".rs") && name !== "tests.rs" && !name.endsWith("_tests.rs")) {
            yield p;
        }
    }
}

const found = [];
for (const file of rustFiles(SRC)) {
    const text = readFileSync(file, "utf8");
    // An inline `#[cfg(test)] mod tests { … }` runs to the end of the file in
    // this codebase; stop counting there.
    const testStart = text.search(/#\[cfg\(test\)\]\s*mod \w+\s*\{/);
    const body = testStart >= 0 ? text.slice(0, testStart) : text;
    body.split("\n").forEach((line, i) => {
        if (/\.register_handler\(/.test(line)) found.push(`${relative(ROOT, file).split(sep).join("/")}:${i + 1}`);
    });
}

if (process.argv.includes("--list")) {
    for (const f of found) console.log(f);
    console.log(`${found.length} untyped handler(s)`);
    process.exit(0);
}

if (found.length > MAX) {
    console.error(`check-untyped-rpc-handlers: ${found.length} register_handler calls, allowed ${MAX}.`);
    console.error("Register new RPC commands with register_typed (typed request and response).");
    console.error("Run with --list to see them all.");
    process.exit(1);
}
if (found.length < MAX) {
    console.error(`check-untyped-rpc-handlers: ${found.length} register_handler calls, below MAX = ${MAX} (thank you).`);
    console.error("Lower MAX in scripts/check-untyped-rpc-handlers.mjs to the new count, so the room can't be taken back.");
    process.exit(1);
}
console.log(`check-untyped-rpc-handlers: ok (${found.length} untyped, none added).`);
