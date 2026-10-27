// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for release-gate.mjs: the decision (evaluateGate) and the reading of
// the five version locations (parseVersions). The GitHub side is a thin wrapper
// over `gh api` and is exercised by the first run of release-core.yml.

import { describe, expect, it } from "vitest";
import { VERSION_FILES, evaluateGate, parseVersions } from "./release-gate.mjs";

const SHA = "0123456789abcdef0123456789abcdef01234567";
const VERSION = "0.60.3";

const releaseFiles = () => [
    ...VERSION_FILES.map((filename) => ({ status: "modified", filename })),
    { status: "removed", filename: ".changesets/1792962560-srv-runtime-stall-log-opz3.md" },
    { status: "removed", filename: ".changesets/1792965462-agent-pane-note-1zbv.md" },
];

const good = (over = {}) => ({
    refType: "tag",
    refName: `v${VERSION}`,
    sha: SHA,
    commit: { status: "behind", subject: `chore: release v${VERSION} (#5621)`, files: releaseFiles() },
    versions: Object.fromEntries(VERSION_FILES.map((f) => [f, VERSION])),
    ...over,
});

const withCommit = (over) => good({ commit: { ...good().commit, ...over } });

describe("evaluateGate", () => {
    it("passes a chore: release commit on main whose files and versions are the release's", () => {
        const r = evaluateGate(good());
        expect(r).toEqual({ ok: true, version: VERSION, reasons: [] });
    });

    it("passes the commit that is main's tip, and a subject with no pull request number", () => {
        expect(evaluateGate(withCommit({ status: "identical" })).ok).toBe(true);
        expect(evaluateGate(withCommit({ subject: `chore: release v${VERSION}` })).ok).toBe(true);
    });

    it("passes the subject as agents write it: the PR-title prefix, then the title, then the PR number", () => {
        for (const subject of [`AgentY@narko: chore: release v${VERSION} (#5621)`, `Maricon-3@charlie: chore: release v${VERSION}`, `a.b_c@host-1.local: chore: release v${VERSION} (#1)`]) {
            expect(evaluateGate(withCommit({ subject })).ok, subject).toBe(true);
        }
    });

    it("passes a release that consumed no changesets at all", () => {
        expect(evaluateGate(withCommit({ files: VERSION_FILES.map((filename) => ({ status: "modified", filename })) })).ok).toBe(true);
    });

    it("refuses a branch run, or a ref that is not a release tag", () => {
        expect(evaluateGate(good({ refType: "branch" })).reasons.join()).toMatch(/not a tag/);
        for (const refName of ["main", "v0.60", "v0.60.3-rc1", "0.60.3", "v0.60.3.1", "vx.y.z", ""]) {
            expect(evaluateGate(good({ refType: "tag", refName })).ok, refName).toBe(false);
        }
    });

    it("refuses a sha that is not a full commit hash", () => {
        expect(evaluateGate(good({ sha: "abc123" })).reasons.join()).toMatch(/not a full commit sha/);
    });

    it("refuses a commit that is not on main", () => {
        for (const status of ["ahead", "diverged", undefined]) {
            expect(evaluateGate(withCommit({ status })).reasons.join(), String(status)).toMatch(/not on main/);
        }
    });

    it("refuses a subject that is not exactly the release's", () => {
        const bad = [
            "chore: release-anything",
            `chore: release v${VERSION} extra`,
            `chore: release v${VERSION}.1`,
            "chore: release v0.60.4",
            `chore: release v${VERSION.replace(/\./g, "x")}`, // dots are literal, not wildcards
            `feat: something (#5621)\n\nchore: release v${VERSION}`,
            `chore: release v${VERSION} (#)`,
            `Chore: release v${VERSION}`,
            `AgentY narko: chore: release v${VERSION}`, // a prefix is <agent>@<host>: and nothing else
            `AgentY@narko:chore: release v${VERSION}`,
            `x: AgentY@narko: chore: release v${VERSION}`,
            `AgentY@narko: feat: chore: release v${VERSION}`,
            "",
        ];
        for (const subject of bad) {
            expect(evaluateGate(withCommit({ subject })).ok, subject).toBe(false);
        }
    });

    it("refuses a commit that changes anything but the release files", () => {
        const extras = [
            { status: "modified", filename: "scripts/release-manifest.mjs" },
            { status: "modified", filename: ".github/workflows/release-core.yml" },
            { status: "added", filename: "src/x.rs" },
            { status: "added", filename: ".changesets/new.md" },
            { status: "modified", filename: ".changesets/1792962560-edit.md" },
            { status: "removed", filename: ".changesets/README.md" },
            { status: "removed", filename: ".changesets/sub/dir.md" },
            { status: "removed", filename: "Cargo.toml" },
            { status: "renamed", filename: "package.json" },
        ];
        for (const extra of extras) {
            const r = evaluateGate(withCommit({ files: [...releaseFiles(), extra] }));
            expect(r.ok, JSON.stringify(extra)).toBe(false);
            expect(r.reasons.join()).toContain(extra.filename);
        }
    });

    it("refuses a commit that does not modify all five version files", () => {
        for (const name of VERSION_FILES) {
            const files = releaseFiles().filter((f) => f.filename !== name);
            const r = evaluateGate(withCommit({ files }));
            expect(r.reasons.join(), name).toContain(`does not modify ${name}`);
        }
    });

    it("refuses when any of the five locations is at another version", () => {
        for (const name of VERSION_FILES) {
            const versions = { ...good().versions, [name]: "0.60.2" };
            const r = evaluateGate(good({ versions }));
            expect(r.ok, name).toBe(false);
            expect(r.reasons.join()).toContain(`${name} is at version '0.60.2'`);
        }
    });

    it("reports every reason, not only the first, and refuses when the commit could not be read", () => {
        const r = evaluateGate(good({ refType: "branch", commit: undefined, versions: undefined }));
        expect(r.ok).toBe(false);
        expect(r.reasons.join()).toMatch(/not a tag.*could not be read/);
        const many = evaluateGate(withCommit({ status: "ahead", subject: "x", files: [{ status: "added", filename: "a" }] }));
        expect(many.reasons.length).toBeGreaterThanOrEqual(8);
    });
});

describe("parseVersions", () => {
    const texts = {
        "package.json": JSON.stringify({ name: "agentmux", version: "0.60.3" }),
        "package-lock.json": JSON.stringify({ name: "agentmux", version: "0.60.3", lockfileVersion: 3 }),
        "Cargo.toml": '[workspace]\nmembers = ["a"]\n\n[workspace.package]\nversion = "0.60.3"\nedition = "2021"\n',
        "VERSION_HISTORY.md": "# Version History\n\n## 0.60.3 - 2026-10-27\n\n- x\n\n## 0.60.2 - 2026-10-26\n",
        "Cargo.lock": 'name = "other"\nversion = "9.9.9"\n\n[[package]]\nname = "agentmux-cef"\nversion = "0.60.3"\ndependencies = []\n',
    };

    it("reads the five locations the way the old verify job did", () => {
        expect(parseVersions(texts)).toEqual({
            "package.json": "0.60.3",
            "package-lock.json": "0.60.3",
            "Cargo.toml": "0.60.3",
            "VERSION_HISTORY.md": "0.60.3",
            "Cargo.lock": "0.60.3",
        });
    });

    it("leaves a location undefined (and so a mismatch) when it cannot be found", () => {
        const v = parseVersions({ ...texts, "Cargo.lock": 'name = "other"\nversion = "1.0.0"\n', "VERSION_HISTORY.md": "no headings\n" });
        expect(v["Cargo.lock"]).toBeUndefined();
        expect(v["VERSION_HISTORY.md"]).toBeUndefined();
        const r = evaluateGate(good({ versions: v }));
        expect(r.ok).toBe(false);
    });
});
