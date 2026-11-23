#!/usr/bin/env node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Known-vulnerability check for the npm packages: `npm audit` against package-lock.json, minus the advisories
 * listed in .github/npm-audit-ignore.json (each with a reason and a review date, kept honest by
 * scripts/advisory-ignores.mjs).
 *
 * Exits 0 when every advisory found is listed, 1 when one is not (or, with `--expired=fail`, when a listed one is
 * past its review date), and 2 when the audit itself could not be read: an audit that did not run is never green.
 * The whole list, listed or not, is printed (and appended to $GITHUB_STEP_SUMMARY when set).
 *
 * Usage: npm-audit-gate.mjs [--expired=fail|warn] [--audit-json <file>] [--ignore <file>]
 *   --audit-json   read `npm audit --json` output from a file instead of running npm (tests, local use)
 */

import { appendFileSync, readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { isMain } from "./lib/is-main.mjs";
import { isExpired, readNpmIgnores } from "./advisory-ignores.mjs";

/** The advisory id npm gives a finding: the GHSA id in its URL, else the numeric source id. */
export function advisoryId(via) {
    const ghsa = /\/(GHSA-[0-9a-z-]+)\/?$/i.exec(String(via?.url ?? ""));
    if (ghsa) return ghsa[1];
    return via?.source !== undefined ? String(via.source) : "";
}

/**
 * The distinct advisories in `npm audit --json` output, each with the packages it reaches:
 * [{ id, severity, title, packages: [name], range }]. Throws when the output isn't an audit report.
 */
export function advisoriesOf(audit) {
    if (!audit || typeof audit !== "object" || audit.error || typeof audit.vulnerabilities !== "object" || audit.vulnerabilities === null) {
        const why = audit && typeof audit === "object" && audit.error ? `${audit.error.code ?? ""} ${audit.error.summary ?? ""}`.trim() : "not an audit report";
        throw new Error(`the audit did not produce a report (${why})`);
    }
    const byId = new Map();
    for (const [name, v] of Object.entries(audit.vulnerabilities)) {
        for (const via of Array.isArray(v?.via) ? v.via : []) {
            if (via === null || typeof via !== "object") continue; // a package that only reaches another vulnerable one
            const id = advisoryId(via);
            if (id === "") throw new Error(`an advisory on ${name} has no id`);
            const found = byId.get(id) ?? { id, severity: via.severity ?? "unknown", title: via.title ?? "", packages: new Set(), range: via.range ?? "" };
            found.packages.add(via.name ?? name);
            byId.set(id, found);
        }
    }
    return [...byId.values()].map((a) => ({ ...a, packages: [...a.packages].sort() })).sort((a, b) => a.id.localeCompare(b.id));
}

/**
 * Compare the advisories with the ignore list. Returns { rows, failing, expired }: every advisory with whether it
 * is ignored, the ones that are not, and the ignored ones past their review date.
 */
export function evaluateAudit(advisories, ignores, today) {
    const byId = new Map(ignores.map((i) => [i.id, i]));
    const rows = advisories.map((a) => {
        const ignore = byId.get(a.id);
        return { ...a, ignored: ignore !== undefined, reason: ignore?.reason ?? "", until: ignore?.until ?? "" };
    });
    return {
        rows,
        failing: rows.filter((r) => !r.ignored),
        expired: rows.filter((r) => r.ignored && isExpired(r.until, today)),
    };
}

function markdown(rows) {
    if (rows.length === 0) return "No known advisories in package-lock.json.\n";
    const lines = ["| Advisory | Severity | Packages | Status |", "| --- | --- | --- | --- |"];
    for (const r of rows) {
        lines.push(`| ${r.id} | ${r.severity} | ${r.packages.join(", ")} | ${r.ignored ? `ignored until ${r.until}` : "**not ignored**"} |`);
    }
    return `${lines.join("\n")}\n`;
}

function runAudit() {
    // A registry that can't be reached is retried once: a blip should not be a red pull request.
    let last = null;
    for (let attempt = 0; attempt < 2; attempt++) {
        const out = spawnSync("npm", ["audit", "--json", "--package-lock-only"], { encoding: "utf8", shell: process.platform === "win32", maxBuffer: 64 * 1024 * 1024 });
        try {
            const parsed = JSON.parse(out.stdout);
            advisoriesOf(parsed); // reads it as a report, or throws
            return parsed;
        } catch (e) {
            last = e;
        }
    }
    throw last;
}

if (isMain(import.meta.url)) {
    const args = process.argv.slice(2);
    const value = (flag) => {
        const at = args.indexOf(flag);
        return at >= 0 ? args[at + 1] : undefined;
    };
    const expired = args.find((a) => a.startsWith("--expired="))?.slice("--expired=".length) ?? "warn";
    let rows;
    let failing;
    let expiredRows;
    try {
        const ignores = readNpmIgnores(readFileSync(value("--ignore") ?? ".github/npm-audit-ignore.json", "utf8"));
        const audit = value("--audit-json") ? JSON.parse(readFileSync(value("--audit-json"), "utf8")) : runAudit();
        ({ rows, failing, expired: expiredRows } = evaluateAudit(advisoriesOf(audit), ignores, new Date().toISOString().slice(0, 10)));
    } catch (e) {
        console.error(`npm-audit-gate: ${e.message}`);
        process.exit(2);
    }
    const table = markdown(rows);
    process.stdout.write(table);
    if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, `### npm advisories\n\n${table}\n`);
    for (const r of expiredRows) console.error(`npm-audit-gate: ${expired === "fail" ? "error" : "warning"}: ${r.id}'s review date ${r.until} has passed`);
    for (const r of failing) console.error(`npm-audit-gate: ${r.id} (${r.severity}) in ${r.packages.join(", ")} is not ignored: ${r.title}`);
    process.exit(failing.length > 0 || (expired === "fail" && expiredRows.length > 0) ? 1 : 0);
}
