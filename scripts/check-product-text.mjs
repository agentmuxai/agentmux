#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-product-text.mjs — CI gate: text AgentMux ships to every user does not name
// the setup of one deployment.
//
// THE RULE. What a user's agents are told on their first start (the seed agents and
// their skills, the operator-config entries, the built-in skills) goes to every
// install, and most installs have none of our own tooling, hosts or repositories.
// Text there that says "run `gh-agent auth status`" or points at a private repository
// is wrong for them, and tells them things about us. This reads those files and fails
// on a deny-list term, naming the file, the line and the term.
//
// Where the text needs a term (a tool named only for the machines that have it), it
// is allowed with an entry in ALLOW below: the exact wording, and a reason. An entry
// that no longer matches anything fails too, so the list cannot go stale.
//
// What is scanned: SEED_FILES (the shipped manifests, whose skills and prompts live in
// them) and USER_FACING_FILES (shipped text kept in code). A listed file that does not
// exist fails, so a rename cannot take a file out of the rule unnoticed. To bring more
// text under the rule, add its file to one of them.
//
// Usage:
//   node scripts/check-product-text.mjs
// Exit 0 = clean, exit 1 = a violation (or a stale allow entry) was found.

import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { isMain } from "./lib/is-main.mjs";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

/** Names of our own setup. Matched case-insensitively, as whole words. */
export const DENY = [
    "a5af",
    "shared-infrastructure",
    "gh-agent",
    "secrets escalate",
    "muxreview",
    "reagent",
    "agentmux-cloud",
];

/** The shipped manifests. */
export const SEED_FILES = [
    "crates/srv/agent-seed.json",
    "crates/srv/operator-config-seed.json",
    "crates/srv/src/config/starter-mcp-servers.json",
];

/** Shipped text that lives in code. */
export const USER_FACING_FILES = [
    "crates/srv/src/migrations/m0015_seed_starter_skills.rs",
    "crates/srv/src/backend/bundle_import_skill_text.rs",
];

/**
 * Wording that may name a term, exactly as it appears in the file (a raw substring,
 * so JSON escapes count), and why. Each entry is removed from the text before the
 * deny-list is read, and must match at least once.
 */
export const ALLOW = [
    {
        file: "crates/srv/agent-seed.json",
        text: "if a `gh-agent` command is installed here, use `gh-agent auth status` instead, and run `gh-agent doctor` if it fails",
        reason: "The setup check names gh-agent only as the alternative on a machine that has it; every other machine is told to use gh.",
    },
    {
        file: "crates/srv/agent-seed.json",
        text: "If neither `gh` nor `gh-agent` is installed",
        reason: "The same conditional: the N/A case is for a machine with neither CLI.",
    },
];

const escapeRe = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
// A whole name. A JSON escape ("\\n", "\\t", "\\"") before it is a boundary: its letter is not part of a word.
const termRe = (term) => new RegExp(`(?:(?<![\\w-])|(?<=\\\\[nrtbf"\\\\/]))${escapeRe(term)}(?![\\w-])`, "gi");

const lineAt = (text, index) => text.slice(0, index).split("\n").length;

/**
 * Violations in one file: `{ path, line, term, text }`, line 1-based. `allow` is the
 * entries for this file; each one it used is added to `used`.
 */
export function checkText(path, text, allow = ALLOW.filter((a) => a.file === path), used = new Set()) {
    // Blank out allowed wording, keeping every offset (and so every line number).
    let scrubbed = text;
    for (const entry of allow) {
        const re = new RegExp(escapeRe(entry.text), "g");
        scrubbed = scrubbed.replace(re, (m) => {
            used.add(entry);
            return " ".repeat(m.length);
        });
    }
    const out = [];
    for (const term of DENY) {
        for (const m of scrubbed.matchAll(termRe(term))) {
            const line = lineAt(scrubbed, m.index);
            out.push({ path, line, term, text: text.split("\n")[line - 1].trim().slice(0, 160) });
        }
    }
    return out.sort((a, b) => a.line - b.line);
}

/** Every file the rule covers, repo-relative. */
export function targets() {
    return [...SEED_FILES, ...USER_FACING_FILES];
}

/** Violations across the repo, the listed files that do not exist, and the allow entries that matched nothing. */
export function run(root = REPO_ROOT) {
    const used = new Set();
    const violations = [];
    const missing = targets().filter((f) => !existsSync(join(root, f)));
    for (const path of targets().filter((f) => !missing.includes(f))) {
        violations.push(...checkText(path, readFileSync(join(root, path), "utf8"), ALLOW.filter((a) => a.file === path), used));
    }
    return { violations, missing, stale: ALLOW.filter((a) => !used.has(a)) };
}

function main() {
    const { violations, missing, stale } = run();
    for (const v of violations) {
        console.error(`${v.path}:${v.line}: "${v.term}" in text every user gets: ${v.text}`);
    }
    for (const f of missing) {
        console.error(`${f}: listed in scripts/check-product-text.mjs but missing (renamed or moved? update the list)`);
    }
    for (const a of stale) {
        console.error(`${a.file}: allow entry matches nothing (remove it): ${a.text.slice(0, 80)}`);
    }
    if (violations.length || missing.length || stale.length) {
        console.error(
            "\nText AgentMux ships to every user must not name one deployment's own setup. Reword it generically, or, " +
                "where a term is needed for the machines that have it, add an ALLOW entry (exact wording and a reason) to scripts/check-product-text.mjs."
        );
        process.exit(1);
    }
    console.log(`check-product-text: ${targets().length} files clean`);
}

if (isMain(import.meta.url)) main();
