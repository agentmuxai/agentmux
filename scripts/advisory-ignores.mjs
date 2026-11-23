#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The advisory ignore lists stay honest: every ignored advisory has a reason and a review date.
 *
 * Two lists exist, one per ecosystem:
 *   deny.toml                   `ignore = [ { id = "RUSTSEC-...", reason = "...; review by YYYY-MM-DD" }, ... ]`
 *                               (cargo-deny, the Rust workspace), one entry per line
 *   .github/npm-audit-ignore.json
 *                               `{ "ignore": [ { "id": "GHSA-...", "reason": "...", "until": "YYYY-MM-DD" } ] }`
 *                               (scripts/npm-audit-gate.mjs, the npm packages)
 *
 * An entry with no id, no reason or no valid date is always refused. An entry whose date has passed is refused
 * with `--expired=fail` (the nightly) and only reported with `--expired=warn` (the default, a pull request), so a
 * date running out never turns an unrelated pull request red.
 *
 * Usage: advisory-ignores.mjs [--expired=fail|warn] [--root <dir>]
 */

import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { isMain } from "./lib/is-main.mjs";

const DATE = /^\d{4}-\d{2}-\d{2}$/;
const REVIEW_BY = /review by (\d{4}-\d{2}-\d{2})/i;

/** The `ignore` entries of a deny.toml, one `{ id = "...", reason = "..." }` per line, as { id, reason, until, line }. */
export function readDenyIgnores(text) {
    const entries = [];
    const lines = String(text ?? "").split(/\r?\n/);
    let inIgnore = false;
    lines.forEach((raw, i) => {
        const line = raw.trim();
        if (/^ignore\s*=\s*\[/.test(line)) inIgnore = true;
        if (!inIgnore) return;
        const id = /\bid\s*=\s*"([^"]*)"/.exec(line);
        if (id) {
            const reason = /\breason\s*=\s*"([^"]*)"/.exec(line)?.[1] ?? "";
            entries.push({ id: id[1], reason, until: REVIEW_BY.exec(reason)?.[1] ?? "", line: i + 1 });
        } else if (/^"[^"]+"\s*,?$/.test(line)) {
            // A bare id with no table: no reason, no date.
            entries.push({ id: line.replace(/[",\s]/g, ""), reason: "", until: "", line: i + 1 });
        }
        if (line.startsWith("]")) inIgnore = false;
    });
    return entries;
}

/** The entries of an npm ignore file, as { id, reason, until }. Unparseable input is an error, never an empty list. */
export function readNpmIgnores(text) {
    const parsed = JSON.parse(text);
    if (!parsed || !Array.isArray(parsed.ignore)) throw new Error('the file must be { "ignore": [ ... ] }');
    return parsed.ignore.map((e, i) => ({
        id: typeof e?.id === "string" ? e.id : "",
        reason: typeof e?.reason === "string" ? e.reason : "",
        until: typeof e?.until === "string" ? e.until : "",
        line: i + 1,
    }));
}

/** Whether `until` (YYYY-MM-DD) is before `today` (YYYY-MM-DD). */
export function isExpired(until, today) {
    return DATE.test(until) && until < today;
}

/**
 * Check one list. Returns { errors, warnings }: an error is a missing id, reason or date (or, with
 * `expired: "fail"`, a date that has passed); a warning is a date that has passed under `expired: "warn"`.
 */
export function checkIgnores(entries, { today, expired = "warn", source = "list" }) {
    const errors = [];
    const warnings = [];
    const seen = new Set();
    for (const e of entries) {
        const at = `${source}:${e.line} ${e.id || "(no id)"}`;
        if (e.id === "") errors.push(`${at}: an entry has no id`);
        if (seen.has(e.id)) errors.push(`${at}: listed twice`);
        seen.add(e.id);
        if (e.reason.trim() === "") errors.push(`${at}: no reason`);
        if (!DATE.test(e.until)) {
            errors.push(`${at}: no review date (YYYY-MM-DD)`);
        } else if (isExpired(e.until, today)) {
            (expired === "fail" ? errors : warnings).push(`${at}: review date ${e.until} has passed`);
        }
    }
    return { errors, warnings };
}

function today() {
    return new Date().toISOString().slice(0, 10);
}

if (isMain(import.meta.url)) {
    const args = process.argv.slice(2);
    const flag = args.find((a) => a.startsWith("--expired="))?.slice("--expired=".length) ?? "warn";
    if (flag !== "fail" && flag !== "warn") {
        console.error("advisory-ignores: --expired must be fail or warn");
        process.exit(2);
    }
    const rootAt = args.indexOf("--root");
    const root = rootAt >= 0 ? args[rootAt + 1] : process.cwd();
    const lists = [
        ["deny.toml", (t) => readDenyIgnores(t)],
        [".github/npm-audit-ignore.json", (t) => readNpmIgnores(t)],
    ];
    let failed = false;
    for (const [file, read] of lists) {
        const path = join(root, file);
        if (!existsSync(path)) {
            console.error(`advisory-ignores: ${file} is missing`);
            failed = true;
            continue;
        }
        let entries;
        try {
            entries = read(readFileSync(path, "utf8"));
        } catch (e) {
            console.error(`advisory-ignores: ${file} can't be read: ${e.message}`);
            failed = true;
            continue;
        }
        const { errors, warnings } = checkIgnores(entries, { today: today(), expired: flag, source: file });
        for (const w of warnings) console.warn(`advisory-ignores: warning: ${w}`);
        for (const e of errors) console.error(`advisory-ignores: ${e}`);
        if (errors.length > 0) failed = true;
        else console.log(`advisory-ignores: ${file}: ${entries.length} ignored advisor${entries.length === 1 ? "y" : "ies"}, each with a reason and a date`);
    }
    process.exit(failed ? 1 : 0);
}
