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
// registrations; by 2026-10-09 there were 60
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

const MAX = 60;

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

// Where the item opened by the `{` at `open` ends: the index of its `}`.
// Braces inside comments, strings, raw strings and char literals don't count.
function closingBrace(text, open) {
    let depth = 0;
    for (let i = open; i < text.length; i++) {
        const c = text[i];
        if (c === "/" && text[i + 1] === "/") {
            i = text.indexOf("\n", i);
            if (i < 0) return text.length - 1;
        } else if (c === "/" && text[i + 1] === "*") {
            i = text.indexOf("*/", i + 2) + 1;
            if (i <= 0) return text.length - 1;
        } else if (c === "r" && /^r#*"/.test(text.slice(i, i + 8)) && !/\w/.test(text[i - 1] ?? "")) {
            const hashes = text.slice(i + 1).match(/^#*/)[0];
            i = text.indexOf(`"${hashes}`, i + hashes.length + 2) + hashes.length;
            if (i < hashes.length) return text.length - 1;
        } else if (c === '"') {
            for (i++; i < text.length && text[i] !== '"'; i++) if (text[i] === "\\") i++;
        } else if (c === "'" && /^'(?:\\.|[^\\'])'/.test(text.slice(i, i + 4))) {
            i = text.indexOf("'", i + 2);
        } else if (c === "{") {
            depth++;
        } else if (c === "}" && --depth === 0) {
            return i;
        }
    }
    return text.length - 1;
}

// The value of a `cfg` predicate in a build without `test`: false, true, or
// null when it depends on something else (a target, a feature, …). Three-valued,
// so `all(any(test, debug_assertions), unix)` is null, not false.
function cfgWithoutTest(expr) {
    let i = 0;
    const skip = () => {
        while (/\s/.test(expr[i] ?? "")) i++;
    };
    const parse = () => {
        skip();
        const name = expr.slice(i).match(/^\w+/)?.[0];
        if (!name) throw new Error(`cfg: expected a predicate in "${expr}"`);
        i += name.length;
        skip();
        if (expr[i] === "=") {
            // key = "value": never `test`.
            i++;
            skip();
            i = expr.indexOf('"', i + 1) + 1;
            return null;
        }
        if (expr[i] !== "(") return name === "test" ? false : null;
        i++;
        const args = [];
        for (skip(); expr[i] !== ")"; skip()) {
            args.push(parse());
            skip();
            if (expr[i] === ",") i++;
        }
        i++;
        if (name === "not") return args[0] === null ? null : !args[0];
        if (name === "all") return args.includes(false) ? false : args.every((a) => a === true) ? true : null;
        if (name === "any") return args.includes(true) ? true : args.every((a) => a === false) ? false : null;
        throw new Error(`cfg: unknown operator ${name}`);
    };
    return parse();
}

// The file with every inline test-only module blanked out: `mod x { … }` under
// a `#[cfg(…)]` that can't hold without `test` (`test`, `all(test, …)`; not
// `not(test)`, `any(test, …)` or `all(any(test, x), …)`). Line breaks are kept,
// so line numbers still match.
function withoutTestMods(text) {
    const head = /#\[cfg\(([^\]]*)\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod \w+\s*\{/g;
    let out = "";
    let from = 0;
    for (let m = head.exec(text); m; m = head.exec(text)) {
        if (cfgWithoutTest(m[1]) !== false) continue;
        const end = closingBrace(text, m.index + m[0].length - 1);
        out += text.slice(from, m.index) + text.slice(m.index, end + 1).replace(/[^\n]/g, " ");
        from = end + 1;
        head.lastIndex = from;
    }
    return out + text.slice(from);
}

const found = [];
for (const file of rustFiles(SRC)) {
    const lines = withoutTestMods(readFileSync(file, "utf8")).split("\n");
    lines.forEach((line, i) => {
        if (!line.trim().startsWith("//") && /\.register_handler\(/.test(line)) {
            found.push(`${relative(ROOT, file).split(sep).join("/")}:${i + 1}`);
        }
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
