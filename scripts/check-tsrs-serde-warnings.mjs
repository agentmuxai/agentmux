#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Fails when ts-rs can't parse a serde attribute on a #[derive(TS)] type
// (docs/specs/SPEC_TSRS_SERDE_ATTRIBUTE_WARNINGS_2026_10_13.md, #4802).
//
// ts-rs prints `failed to parse serde attribute` and then ignores the item. Before ts-rs 12 it
// ignored the whole attribute, so a `rename` beside an unknown item was silently lost and the
// generated TypeScript named a key the server never sends; that caused the "My Agents empty"
// incident. A warning here means the generated type may not match the wire, so it fails the
// build, unless its item is on ALLOWED with the reason it can't change the TypeScript.
//
//   node scripts/check-tsrs-serde-warnings.mjs              runs `cargo check -p agentmux-srv --tests`
//   node scripts/check-tsrs-serde-warnings.mjs --log FILE   reads that build output instead
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

/** Items ts-rs warns on that can't change the generated TypeScript, each with why. */
export const ALLOWED = [
    { item: /^alias = /, why: "deserialize-only: it adds a name srv accepts, never one it sends" },
];

/** The attribute items ts-rs couldn't parse, from cargo's output. Each warning shows the item on
 *  the `|` line after it. */
export function unparsedItems(output) {
    const lines = output.split(/\r?\n/);
    const items = [];
    lines.forEach((line, i) => {
        if (!/warning: failed to parse serde attribute/.test(line)) return;
        for (const next of lines.slice(i + 1, i + 5)) {
            const m = /^\s*\|\s+(\S.*)$/.exec(next);
            if (m) {
                items.push(m[1].trim());
                return;
            }
        }
        items.push("(no attribute shown)");
    });
    return items;
}

/** The items not on ALLOWED. */
export function disallowed(items) {
    return items.filter((item) => !ALLOWED.some((a) => a.item.test(item)));
}

function main() {
    const at = process.argv.indexOf("--log");
    let output;
    if (at >= 0) {
        output = readFileSync(process.argv[at + 1], "utf8");
    } else {
        const run = spawnSync("cargo", ["check", "-p", "agentmux-srv", "--tests", "--message-format", "short"], { encoding: "utf8", maxBuffer: 1 << 28 });
        output = `${run.stdout}\n${run.stderr}`;
        if (run.status !== 0) {
            console.error("check-tsrs-serde-warnings: cargo check failed; fix the build first.");
            process.exit(1);
        }
    }
    const items = unparsedItems(output);
    const bad = disallowed(items);
    if (bad.length) {
        console.error(`check-tsrs-serde-warnings: ts-rs ignored ${bad.length} serde attribute item(s):`);
        for (const b of bad) console.error(`  ${b}`);
        console.error("The generated TypeScript may not match the wire. Write the attribute so ts-rs parses it, or");
        console.error("add #[ts(...)] that says what the field is (optional, rename, type). See the spec named above.");
        process.exit(1);
    }
    console.log(`check-tsrs-serde-warnings: ok (${items.length} allowed: ${[...new Set(items)].join(", ") || "none"})`);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === resolve(process.argv[1])) main();
