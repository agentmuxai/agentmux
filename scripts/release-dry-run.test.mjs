// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Pins the output of `scripts/release.sh --dry-run` byte for byte, over a folder
// of sample changesets (major, minor, patch, an unknown type, a missing type,
// extra prose, CRLF line ends, an empty file ...).
//
// WHY this exists: release.sh used to start one awk per changeset, twice (about
// 450 process starts for 150 changesets, minutes on Windows); it now reads every
// changeset in one awk process. scripts/release-dry-run-reference.sh is the
// per-file reader, frozen. Each scenario runs both over the same changesets and
// requires the same exit status, stdout and stderr, byte for byte: the same
// warnings, notes and entries in the same order. Both run under the same awk, so
// platform differences (gawk on Windows folds CRLF, others do not) cancel out.
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll, describe, expect, it } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));
const releaseSh = join(here, "release.sh");
const referenceSh = join(here, "release-dry-run-reference.sh");

const cs = (type, ...body) => `---\ntype: ${type}\n---\n${body.join("\n")}\n`;

// Files are listed in name order: the script walks them in glob order.
const FIXTURES = {
    "1000-patch-first.md": cs("patch", "First patch entry"),
    "1001-unknown-before.md": cs("banana", "Unknown type before any major"),
    "1002-minor.md": cs("minor", "A minor entry", "", "Extra prose for the reviewer.", "More prose."),
    "1003-blank-lead.md": "---\ntype: patch\n---\n\n   \n  Title after blank lines  \nsecond line\n",
    "1004-no-type.md": "---\nsummary: no type here\n---\nNo type line\n",
    "1005-empty-type.md": "---\ntype:\n---\nEmpty type value\n",
    "1006-type-no-space.md": "---\ntype:patch\n---\nType without a space\n",
    "1007-crlf.md": "---\r\ntype: patch\r\n---\r\nCRLF entry\r\n",
    "1008-empty.md": "",
    "1009-special.md": cs("patch", "Fix `$HOME` and *globs* \\ back\\slash\ttab & \"quotes\" 'single'"),
    "1010-body-fences.md": "---\ntype: minor\n---\nBody title\n---\ntype: major\nafter a third fence\n",
    "1011-type-in-body.md": "---\nnote: x\n---\ntype: major\nthe type line is in the body\n",
    "1012-extra-fields.md": "---\ntype: patch trailing words\nother: y\n---\nExtra words after the type\n",
    "1013-no-newline-at-end.md": "---\ntype: patch\n---\nNo final newline",
    "1014-only-frontmatter.md": "---\ntype: patch\n---\n",
    "1015-tabs.md": "---\ntype:\tminor\t\n---\n\tTabbed title\n",
    "1020-major.md": cs("major", "A major entry"),
    "1021-unknown-after-major.md": cs("banana", "Unknown type after the major"),
    "1022-minor-last.md": cs("minor", "Last entry"),
};

const SCENARIOS = {
    "all, default": { files: Object.keys(FIXTURES), args: ["--dry-run"] },
    "all, --as patch": { files: Object.keys(FIXTURES), args: ["--dry-run", "--as", "patch"] },
    "no major, default": {
        files: Object.keys(FIXTURES).filter((f) => !/-major\.md$/.test(f) && f !== "1010-body-fences.md"),
        args: ["--dry-run"],
    },
    "no major, --as patch": {
        files: Object.keys(FIXTURES).filter((f) => !/-major\.md$/.test(f) && f !== "1010-body-fences.md"),
        args: ["--dry-run", "--as", "patch"],
    },
    "patch only, --as patch": { files: ["1000-patch-first.md", "1009-special.md"], args: ["--dry-run", "--as", "patch"] },
    "patch only, --as minor": { files: ["1000-patch-first.md", "1009-special.md"], args: ["--dry-run", "--as", "minor"] },
    "crlf only": { files: ["1007-crlf.md"], args: ["--dry-run"] },
    "empty file only": { files: ["1008-empty.md"], args: ["--dry-run"] },
    "a README is not a changeset": { files: ["1000-patch-first.md"], extra: { "README.md": cs("major", "no") }, args: ["--dry-run"] },
    "none": { files: [], args: ["--dry-run"] },
};

const dirs = [];
afterAll(() => {
    for (const d of dirs) rmSync(d, { recursive: true, force: true });
});

function runScenario(script, { files, extra = {}, args }) {
    const dir = mkdtempSync(join(tmpdir(), "release-dry-run-"));
    dirs.push(dir);
    const git = spawnSync("git", ["init", "-q"], { cwd: dir });
    if (git.status !== 0) throw new Error(`git init failed: ${git.stderr}`);
    mkdirSync(join(dir, ".changesets"));
    for (const f of files) writeFileSync(join(dir, ".changesets", f), FIXTURES[f]);
    for (const [f, body] of Object.entries(extra)) writeFileSync(join(dir, ".changesets", f), body);
    const r = spawnSync("bash", [script, ...args], { cwd: dir, encoding: "utf8" });
    return { status: r.status, stdout: r.stdout, stderr: r.stderr };
}

// The per-file reference is slow on a loaded Windows host (a process start per
// changeset); give each scenario room.
const TIMEOUT_MS = 120_000;

describe("release.sh --dry-run output", () => {
    for (const [name, scenario] of Object.entries(SCENARIOS)) {
        it(`matches the per-file reader byte for byte: ${name}`, { timeout: TIMEOUT_MS }, () => {
            const expected = runScenario(referenceSh, scenario);
            const actual = runScenario(releaseSh, scenario);
            expect(actual).toEqual(expected);
        });
    }

    it("covers the cases the reader has to get right", () => {
        const r = runScenario(releaseSh, SCENARIOS["all, default"]);
        expect(r.status).toBe(0);
        expect(r.stderr).toContain("unknown type 'banana'");
        expect(r.stderr).toContain("Release type: major");
        // An unknown type after the first major is never reached (the loop stops).
        expect(r.stderr).not.toContain("1021-unknown-after-major.md has unknown type");
    });
});
