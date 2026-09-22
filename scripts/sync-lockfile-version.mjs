#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Update package-lock.json's version field(s) in place, WITHOUT reformatting
 * the rest of the file.
 *
 * WHY this exists: the inline `node -e` this replaces (in
 * scripts/bump-wrapper.sh) always wrote `JSON.stringify(lock, null, 2)` —
 * hardcoded 2-space, regardless of the file's actual indentation. This repo's
 * committed package-lock.json is 4-space. Every release bump therefore
 * reformatted all ~6000 lines of it to 2-space, producing a ~24,000-line diff
 * (roughly 12,000 changed lines × 2 for the unified-diff +/-) that carried
 * zero dependency changes — first hit for real on PR #3494/#3495's release.
 *
 * Fix: detect the file's own indentation before parsing, and write it back
 * with that same indentation. If the file can't be sniffed for an indent unit
 * (e.g. it's a single-line/minified JSON, which never happens in practice for
 * a real package-lock.json but is a cheap case to handle), fall back to the
 * project's actual 4-space convention rather than npm's 2-space default —
 * matching the file this script exists to serve, not some other project's.
 *
 * Usage:
 *   node scripts/sync-lockfile-version.mjs <version> [lockfile-path]
 *   node scripts/sync-lockfile-version.mjs 0.56.11
 *   node scripts/sync-lockfile-version.mjs 0.56.11 package-lock.json
 *
 * lockfile-path defaults to package-lock.json in the current directory.
 * Exits non-zero (with a message on stderr) if the file can't be read/parsed
 * or the version arg is missing — matching the ERROR-and-exit-1 contract
 * bump-wrapper.sh already relies on.
 */

import { readFileSync, writeFileSync } from "node:fs";

const DEFAULT_INDENT = 4; // this repo's actual package-lock.json convention

/**
 * Sniff the indentation unit from a JSON text's first indented line.
 *
 * A real package-lock.json's first object-key line (depth 1, e.g. `"name":
 * "agentmux",`) is indented by exactly one level, so its leading whitespace
 * IS the per-level indent unit — no depth tracking needed.
 *
 * Returns a number (spaces per level) or the literal tab character, either of
 * which JSON.stringify's third argument accepts directly. Falls back to
 * DEFAULT_INDENT when no indented line is found (empty object, or minified
 * single-line JSON — doesn't happen for a real lockfile, but fails safe
 * rather than throwing).
 */
export function detectIndent(text, fallback = DEFAULT_INDENT) {
    for (const line of text.split(/\r?\n/)) {
        const tabs = line.match(/^(\t+)\S/);
        if (tabs) return "\t";
        const spaces = line.match(/^( +)\S/);
        if (spaces) return spaces[1].length;
    }
    return fallback;
}

/**
 * Parse `text` as a package-lock.json, set its version field(s) to `version`,
 * and re-serialize using the SAME indentation the input already had.
 *
 * Mirrors exactly the two fields the previous inline script updated:
 * top-level `version` and `packages[""].version` (the lockfileVersion-3 shape
 * npm uses for the root package's own entry). Nothing else is touched —
 * dependency entries, `resolved`/`integrity` values, and key ordering all
 * pass through JSON.parse/JSON.stringify unchanged.
 */
export function syncLockfileVersion(text, version) {
    const lock = JSON.parse(text);
    lock.version = version;
    if (lock.packages && lock.packages[""]) {
        lock.packages[""].version = version;
    }
    const indent = detectIndent(text);
    return { text: JSON.stringify(lock, null, indent) + "\n", indent };
}

const isMain =
    process.argv[1] &&
    import.meta.url.endsWith(process.argv[1].replace(/\\/g, "/").split("/").pop());
if (isMain) {
    const [version, lockfilePath = "package-lock.json"] = process.argv.slice(2);
    if (!version) {
        console.error("Usage: sync-lockfile-version.mjs <version> [lockfile-path]");
        process.exit(1);
    }
    let raw;
    try {
        raw = readFileSync(lockfilePath, "utf8");
    } catch (err) {
        console.error(`sync-lockfile-version: could not read ${lockfilePath}: ${err.message}`);
        process.exit(1);
    }
    let result;
    try {
        result = syncLockfileVersion(raw, version);
    } catch (err) {
        console.error(`sync-lockfile-version: ${err.message}`);
        process.exit(1);
    }
    writeFileSync(lockfilePath, result.text);
    const indentDesc = result.indent === "\t" ? "tabs" : `${result.indent}-space`;
    console.error(`sync-lockfile-version: wrote ${lockfilePath} at ${version} (${indentDesc} indent preserved)`);
}
