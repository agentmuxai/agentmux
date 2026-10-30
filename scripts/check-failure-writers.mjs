#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-failure-writers.mjs — CI gate: the pane's failure row is raised and cleared in one place.
//
// THE RULE. Every `FailureObserved` and `FailureCleared` command goes through
// frontend/app/view/agent/failure/failure-writer.ts (`raiseFailure`, `clearFailure`), which names
// who raises it or why it clears, so the reducer can apply the written precedence
// (frontend/app/store/agent-pane-state/failure-precedence.ts). A direct dispatch skips that: it was
// how eight places raised and eight cleared the same failure with the "which wins" rule in comments.
//
// Fails, in frontend/, on `type: "FailureObserved"` or `type: "FailureCleared"` in any file other
// than the writer, the pane-state store (the command types and the reducer) and tests.
//
// Usage: node scripts/check-failure-writers.mjs
// Exit 0 = clean, exit 1 = a direct dispatch was found.

import { readdirSync, readFileSync } from "node:fs";
import { dirname, extname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { isMain } from "./lib/is-main.mjs";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

const WRITER = "frontend/app/view/agent/failure/failure-writer.ts";
const STORE_DIR = "frontend/app/store/agent-pane-state/";
const DISPATCH = /type:\s*["']Failure(Observed|Cleared)["']/;

const isExempt = (path) => path === WRITER || path.startsWith(STORE_DIR) || /\.test\.[cm]?[jt]sx?$/.test(path);

/** Direct dispatches in one file: `{ path, line, text }`, line 1-based, `path` repo-relative with forward slashes. */
export function checkFile(path, text) {
    if (isExempt(path)) return [];
    const out = [];
    text.split("\n").forEach((l, i) => {
        if (DISPATCH.test(l)) out.push({ path, line: i + 1, text: l.trim() });
    });
    return out;
}

function walk(dir, out = []) {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
        if (entry.name === "node_modules" || entry.name === "dist") continue;
        const full = join(dir, entry.name);
        if (entry.isDirectory()) walk(full, out);
        else if ([".ts", ".tsx"].includes(extname(entry.name))) out.push(full);
    }
    return out;
}

/** Every direct dispatch under `root`/frontend. */
export function collect(root = REPO_ROOT) {
    return walk(join(root, "frontend")).flatMap((full) => checkFile(relative(root, full).split(sep).join("/"), readFileSync(full, "utf8")));
}

if (isMain(import.meta.url)) {
    const found = collect();
    if (found.length === 0) {
        console.log("OK: the failure row is raised and cleared only through failure-writer.ts.");
    } else {
        console.error("ERROR: direct FailureObserved / FailureCleared dispatch (use raiseFailure / clearFailure from");
        console.error(`${WRITER}, which names the source or the reason):`);
        for (const v of found) console.error(`  ${v.path}:${v.line}: ${v.text}`);
        process.exit(1);
    }
}
