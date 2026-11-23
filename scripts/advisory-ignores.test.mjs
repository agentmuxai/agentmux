// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for advisory-ignores.mjs: the ignore lists carry a reason and a review date for every advisory.

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { checkIgnores, isExpired, readDenyIgnores, readNpmIgnores } from "./advisory-ignores.mjs";

const repo = join(dirname(fileURLToPath(import.meta.url)), "..");

const DENY = `
[advisories]
version = 2
ignore = [
    { id = "RUSTSEC-2026-0001", reason = "no patched release is published; review by 2027-02-20" },
    { id = "RUSTSEC-2026-0002", reason = "no replacement chosen yet; review by 2026-01-01" },
]
`;

describe("readDenyIgnores", () => {
    it("reads each entry's id, reason and review date", () => {
        expect(readDenyIgnores(DENY)).toEqual([
            { id: "RUSTSEC-2026-0001", reason: "no patched release is published; review by 2027-02-20", until: "2027-02-20", line: 5 },
            { id: "RUSTSEC-2026-0002", reason: "no replacement chosen yet; review by 2026-01-01", until: "2026-01-01", line: 6 },
        ]);
    });

    it("reads a bare id as an entry with no reason and no date", () => {
        const entries = readDenyIgnores('[advisories]\nignore = [\n    "RUSTSEC-2026-0003",\n]\n');
        expect(entries).toEqual([{ id: "RUSTSEC-2026-0003", reason: "", until: "", line: 3 }]);
    });

    it("reads nothing outside the ignore list, and nothing from an empty file", () => {
        expect(readDenyIgnores('[bans]\nskip = [\n    { id = "x" },\n]\n')).toEqual([]);
        expect(readDenyIgnores("")).toEqual([]);
        expect(readDenyIgnores(undefined)).toEqual([]);
    });
});

describe("readNpmIgnores", () => {
    it("reads the entries", () => {
        expect(readNpmIgnores('{"ignore":[{"id":"GHSA-first","reason":"r","until":"2027-01-01"}]}')).toEqual([
            { id: "GHSA-first", reason: "r", until: "2027-01-01", line: 1 },
        ]);
    });

    it("refuses a file that is not a list, rather than reading it as empty", () => {
        expect(() => readNpmIgnores("{}")).toThrow();
        expect(() => readNpmIgnores('{"ignore":"x"}')).toThrow();
        expect(() => readNpmIgnores("not json")).toThrow();
    });

    it("reads an entry with missing fields as incomplete, so the check refuses it", () => {
        const [entry] = readNpmIgnores('{"ignore":[{"id":"GHSA-first"}]}');
        expect(entry).toMatchObject({ reason: "", until: "" });
    });
});

describe("checkIgnores", () => {
    const today = "2026-11-22";
    const entry = (over = {}) => ({ id: "RUSTSEC-2026-0001", reason: "a reason", until: "2027-02-20", line: 1, ...over });

    it("accepts an entry with a reason and a date still ahead", () => {
        expect(checkIgnores([entry()], { today })).toEqual({ errors: [], warnings: [] });
    });

    it("refuses an entry with no reason", () => {
        expect(checkIgnores([entry({ reason: "  " })], { today }).errors.join("\n")).toContain("no reason");
    });

    it("refuses an entry with no date or a date that is not YYYY-MM-DD", () => {
        for (const until of ["", "soon", "2027-2-3", "20270203"]) {
            expect(checkIgnores([entry({ until })], { today }).errors.join("\n"), until).toContain("no review date");
        }
    });

    it("refuses an entry with no id and one listed twice", () => {
        expect(checkIgnores([entry({ id: "" })], { today }).errors.join("\n")).toContain("no id");
        expect(checkIgnores([entry(), entry({ line: 2 })], { today }).errors.join("\n")).toContain("listed twice");
    });

    it("only warns about a date that has passed by default, and refuses it with expired=fail", () => {
        const past = entry({ until: "2026-11-21" });
        expect(checkIgnores([past], { today })).toMatchObject({ errors: [], warnings: [expect.stringContaining("has passed")] });
        expect(checkIgnores([past], { today, expired: "fail" })).toMatchObject({ errors: [expect.stringContaining("has passed")], warnings: [] });
    });

    it("treats the review date itself as still open", () => {
        expect(isExpired("2026-11-22", "2026-11-22")).toBe(false);
        expect(isExpired("2026-11-21", "2026-11-22")).toBe(true);
        expect(isExpired("", "2026-11-22")).toBe(false);
    });
});

describe("the repository's own lists", () => {
    it("deny.toml's ignore entries each have a reason and a review date", () => {
        const entries = readDenyIgnores(readFileSync(join(repo, "deny.toml"), "utf8"));
        expect(entries.length).toBeGreaterThan(0);
        expect(checkIgnores(entries, { today: "2026-01-01", source: "deny.toml" }).errors).toEqual([]);
    });

    it("the npm ignore file reads, and each entry has a reason and a review date", () => {
        const entries = readNpmIgnores(readFileSync(join(repo, ".github", "npm-audit-ignore.json"), "utf8"));
        expect(checkIgnores(entries, { today: "2026-01-01", source: "npm-audit-ignore.json" }).errors).toEqual([]);
    });

    it("the command line passes for the repository's lists", () => {
        const out = spawnSync(process.execPath, [join(repo, "scripts", "advisory-ignores.mjs"), "--root", repo], { encoding: "utf8" });
        expect(out.status, out.stderr).toBe(0);
        expect(out.stdout).toContain("deny.toml");
    });

    it("the command line refuses a list with an entry that has no date", () => {
        const dir = join(repo, "scripts", "test-fixtures", "advisory-ignores-bad");
        const out = spawnSync(process.execPath, [join(repo, "scripts", "advisory-ignores.mjs"), "--root", dir], { encoding: "utf8" });
        expect(out.status).toBe(1);
        expect(out.stderr).toContain("no review date");
    });
});
