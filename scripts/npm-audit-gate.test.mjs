// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for npm-audit-gate.mjs: the npm known-vulnerability check and its ignore list.

import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { advisoriesOf, advisoryId, evaluateAudit } from "./npm-audit-gate.mjs";

const repo = join(dirname(fileURLToPath(import.meta.url)), "..");
const script = join(repo, "scripts", "npm-audit-gate.mjs");

/** An `npm audit --json` report: vitest reaches @vitest/mocker, which carries two advisories; minimatch carries one. */
function report() {
    return {
        auditReportVersion: 2,
        vulnerabilities: {
            vitest: { name: "vitest", severity: "moderate", isDirect: true, via: ["@vitest/mocker"] },
            "@vitest/mocker": {
                name: "@vitest/mocker",
                severity: "moderate",
                via: [
                    { source: 1, name: "@vitest/mocker", title: "first", url: "https://github.com/advisories/GHSA-first", severity: "moderate", range: "<5.0.3" },
                    { source: 2, name: "@vitest/mocker", title: "second", url: "https://github.com/advisories/GHSA-second", severity: "high", range: "<5.0.3" },
                ],
            },
            minimatch: {
                name: "minimatch",
                severity: "moderate",
                via: [{ source: 3, name: "minimatch", title: "third", url: "https://github.com/advisories/GHSA-third", severity: "moderate", range: "<11" }],
            },
            "@typescript-eslint/utils": { name: "@typescript-eslint/utils", severity: "moderate", via: ["minimatch"] },
        },
        metadata: { vulnerabilities: { total: 4 } },
    };
}

describe("advisoryId", () => {
    it("is the GHSA id in the advisory's URL, else the numeric source", () => {
        expect(advisoryId({ url: "https://github.com/advisories/GHSA-first", source: 1 })).toBe("GHSA-first");
        expect(advisoryId({ url: "https://example.org/x", source: 1234 })).toBe("1234");
        expect(advisoryId({})).toBe("");
    });
});

describe("advisoriesOf", () => {
    it("lists each distinct advisory once, with the packages it reaches, and skips packages that only reach another", () => {
        const list = advisoriesOf(report());
        expect(list.map((a) => a.id)).toEqual(["GHSA-first", "GHSA-second", "GHSA-third"]);
        expect(list[1]).toMatchObject({ severity: "high", packages: ["@vitest/mocker"] });
    });

    it("lists no advisory for a clean report", () => {
        expect(advisoriesOf({ vulnerabilities: {} })).toEqual([]);
    });

    it("refuses output that is not an audit report, so an audit that did not run is never green", () => {
        for (const bad of [null, undefined, "x", {}, { error: { code: "ENOAUDIT", summary: "no lock" } }, { vulnerabilities: null }]) {
            expect(() => advisoriesOf(bad), JSON.stringify(bad)).toThrow(/did not produce a report/);
        }
    });

    it("refuses an advisory it cannot name", () => {
        expect(() => advisoriesOf({ vulnerabilities: { a: { via: [{ name: "a", title: "t" }] } } })).toThrow(/no id/);
    });
});

describe("evaluateAudit", () => {
    const today = "2026-11-22";
    const ignore = (id, until = "2027-02-20") => ({ id, reason: "r", until, line: 1 });

    it("fails on an advisory that is not ignored, and only on that one", () => {
        const { rows, failing } = evaluateAudit(advisoriesOf(report()), [ignore("GHSA-first"), ignore("GHSA-second")], today);
        expect(rows).toHaveLength(3);
        expect(failing.map((r) => r.id)).toEqual(["GHSA-third"]);
    });

    it("passes when every advisory is ignored, and prints the whole list either way", () => {
        const all = ["GHSA-first", "GHSA-second", "GHSA-third"].map((id) => ignore(id));
        const { rows, failing } = evaluateAudit(advisoriesOf(report()), all, today);
        expect(failing).toEqual([]);
        expect(rows.every((r) => r.ignored)).toBe(true);
    });

    it("reports an ignored advisory past its review date", () => {
        const { expired } = evaluateAudit(advisoriesOf(report()), [ignore("GHSA-first", "2026-11-21")], today);
        expect(expired.map((r) => r.id)).toEqual(["GHSA-first"]);
    });

    it("does not let an ignore for another advisory hide a new one", () => {
        const { failing } = evaluateAudit(advisoriesOf(report()), [ignore("GHSA-other")], today);
        expect(failing).toHaveLength(3);
    });
});

describe("the command line", () => {
    function run(audit, ignore, extra = []) {
        const dir = mkdtempSync(join(tmpdir(), "npm-audit-gate-"));
        try {
            const auditFile = join(dir, "audit.json");
            const ignoreFile = join(dir, "ignore.json");
            writeFileSync(auditFile, typeof audit === "string" ? audit : JSON.stringify(audit));
            writeFileSync(ignoreFile, JSON.stringify({ ignore: ignore }));
            return spawnSync(process.execPath, [script, "--audit-json", auditFile, "--ignore", ignoreFile, ...extra], { encoding: "utf8" });
        } finally {
            rmSync(dir, { recursive: true, force: true });
        }
    }
    const listed = (id, until = "2099-01-01") => ({ id, reason: "r", until });

    it("exits 1 and names an advisory that is not ignored", () => {
        const out = run(report(), [listed("GHSA-first")]);
        expect(out.status).toBe(1);
        expect(out.stderr).toContain("GHSA-second");
        expect(out.stdout).toContain("**not ignored**");
    });

    it("exits 0 when every advisory is ignored", () => {
        const out = run(report(), ["GHSA-first", "GHSA-second", "GHSA-third"].map((id) => listed(id)));
        expect(out.status, out.stderr).toBe(0);
    });

    it("exits 0 for a clean report", () => {
        expect(run({ vulnerabilities: {} }, []).status).toBe(0);
    });

    it("exits 2 when the audit could not be read, and when the ignore file is not a list", () => {
        expect(run("not json", []).status).toBe(2);
        expect(run({ error: { code: "E", summary: "s" } }, []).status).toBe(2);
        const dir = mkdtempSync(join(tmpdir(), "npm-audit-gate-"));
        try {
            const bad = join(dir, "ignore.json");
            writeFileSync(bad, "{}");
            const clean = join(dir, "audit.json");
            writeFileSync(clean, JSON.stringify({ vulnerabilities: {} }));
            expect(spawnSync(process.execPath, [script, "--audit-json", clean, "--ignore", bad], { encoding: "utf8" }).status).toBe(2);
        } finally {
            rmSync(dir, { recursive: true, force: true });
        }
    });

    it("exits 1 for an ignored advisory past its review date only with --expired=fail", () => {
        const all = ["GHSA-first", "GHSA-second", "GHSA-third"].map((id) => listed(id, "2020-01-01"));
        expect(run(report(), all).status).toBe(0);
        expect(run(report(), all, ["--expired=fail"]).status).toBe(1);
    });
});
