// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { baseRef, changedRows, pathsOf, untrackedFiles } from "./lib/branch-changes.mjs";

// Each test runs git a dozen times; a loaded machine takes a minute for that.
vi.setConfig({ testTimeout: 120_000, hookTimeout: 120_000 });

let root;

const git = (...args) =>
    execFileSync("git", ["-c", "user.name=t", "-c", "user.email=t@example.com", "-c", "commit.gpgsign=false", ...args], {
        cwd: root,
        encoding: "utf8",
    });
const write = (rel, text) => {
    mkdirSync(dirname(join(root, rel)), { recursive: true });
    writeFileSync(join(root, rel), text);
};

beforeEach(() => {
    // CI sets the PR's base, which the fixture repo does not have: on a PR whose base is not main (one stacked on another),
    // `changedRows({ root })` would answer null. The fixture's base is main.
    vi.stubEnv("GITHUB_BASE_REF", "");
    root = mkdtempSync(join(tmpdir(), "branch-changes-"));
    git("init", "-q", "-b", "main");
    write("README.md", "base\n");
    write("docs/specs/OLD.md", "**Status:** active\n".repeat(20));
    git("add", ".");
    git("commit", "-q", "-m", "base");
    git("checkout", "-q", "-b", "feature");
});

afterEach(() => {
    vi.unstubAllEnvs();
    rmSync(root, { recursive: true, force: true });
});

describe("branch-changes", () => {
    it("finds the base branch, and null when there is none", () => {
        expect(baseRef(root, {})).toBe("main");
        expect(baseRef(root, { GITHUB_BASE_REF: "nope" })).toBeNull();
    });

    it("answers the same for a staged file as for a committed one", () => {
        write("docs/specs/NEW.md", "**Status:** draft\n");
        git("add", "docs/specs/NEW.md");
        const staged = changedRows({ root });
        git("commit", "-q", "-m", "add a spec");
        expect(staged).toEqual([["A", "docs/specs/NEW.md"]]);
        expect(changedRows({ root })).toEqual(staged);
    });

    it("counts an edit that is not staged, and leaves out a file git does not track", () => {
        write("README.md", "edited\n");
        write("docs/specs/LOOSE.md", "**Status:** draft\n");
        expect(changedRows({ root })).toEqual([["M", "README.md"]]);
        expect(untrackedFiles(root)).toEqual(["docs/specs/LOOSE.md"]);
        expect(changedRows({ root, untracked: true })).toEqual([["M", "README.md"], ["A", "docs/specs/LOOSE.md"]]);
    });

    it("gives both paths of a rename, and a deletion as D", () => {
        git("mv", "docs/specs/OLD.md", "docs/specs/MOVED.md");
        write("README.md", "x");
        git("rm", "-q", "-f", "README.md");
        const rows = changedRows({ root });
        expect(rows).toContainEqual(["D", "README.md"]);
        expect(rows.find((r) => r[0][0] === "R")?.slice(1)).toEqual(["docs/specs/OLD.md", "docs/specs/MOVED.md"]);
        expect(pathsOf(rows)).toEqual(["docs/specs/MOVED.md"]);
        expect(pathsOf(rows, "D")).toEqual(["README.md"]);
    });

    it("is empty on a branch that changes nothing, and null with no base to compare with", () => {
        expect(changedRows({ root })).toEqual([]);
        expect(changedRows({ root, ref: null })).toBeNull();
    });
});
