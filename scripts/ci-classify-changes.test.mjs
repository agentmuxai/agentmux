// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for ci-classify-changes.mjs.
// Spec: docs/specs/SPEC_CI_PATH_CONDITIONAL_CHECKS_2026_09_21.md §2 (R1-R3, R6).
//
// Why these tests carry more weight than most: the classifier's output feeds a
// job-level `if:`, and GitHub counts a *skipped* job as PASSING for a required
// status check. A wrong answer here does not turn CI red — it puts a green
// check on code nobody compiled. So each rule gets its own adversarial cases,
// and every case that should RUN is written as the assertion, not implied.

import { describe, expect, it } from "vitest";
import { classifyChanges, isDocsOnlyPath } from "./ci-classify-changes.mjs";

describe("isDocsOnlyPath — things that genuinely cannot affect a build", () => {
    it("accepts documentation and licence files", () => {
        expect(isDocsOnlyPath("docs/specs/SPEC_FOO_2026_09_21.md")).toBe(true);
        expect(isDocsOnlyPath("README.md")).toBe(true);
        expect(isDocsOnlyPath("VERSION_HISTORY.md")).toBe(true);
        expect(isDocsOnlyPath("LICENSE")).toBe(true);
        expect(isDocsOnlyPath("NOTICE")).toBe(true);
    });

    it("accepts non-markdown files under docs/, which no build reads", () => {
        // `docs/**` is wholesale-safe: the `docs` job still runs and gates it.
        expect(isDocsOnlyPath("docs/assets/architecture.png")).toBe(true);
        expect(isDocsOnlyPath("docs/reports/REPORT_CI_LANE_BUDGET.txt")).toBe(true);
    });

    it("accepts changeset entries, which only release/packaging scripts read", () => {
        // Verified 2026-09-21: `.changesets/` is consumed by release.sh,
        // package*.sh and nightly-release.yml — none of which run on a PR.
        expect(isDocsOnlyPath(".changesets/patch-1234.md")).toBe(true);
    });
});

describe("isDocsOnlyPath — R3, CI configuration is never documentation", () => {
    // The most dangerous misclassification available, because it is
    // self-concealing: a PR editing the test setup must not skip the tests it
    // edits. R3 is checked BEFORE the docs patterns, so `.md` cannot launder a
    // path out of these directories.
    it("rejects everything under the CI-controlling directories", () => {
        expect(isDocsOnlyPath(".github/workflows/ci-pr.yml")).toBe(false);
        expect(isDocsOnlyPath("scripts/ci-classify-changes.mjs")).toBe(false);
        expect(isDocsOnlyPath("tools/muxlog/index.mjs")).toBe(false);
    });

    it("rejects markdown inside those directories too", () => {
        expect(isDocsOnlyPath(".github/ISSUE_TEMPLATE/bug_report.md")).toBe(false);
        expect(isDocsOnlyPath("scripts/README.md")).toBe(false);
        expect(isDocsOnlyPath("tools/muxlog/README.md")).toBe(false);
    });

    it("does NOT treat `.changesets/` as CI config, and that is load-bearing", () => {
        // reagentx P2 on #3491 spotted a dead `.changeset/` (singular) entry in
        // NEVER_DOCS_PREFIXES and read it as a typo for the real `.changesets/`.
        // Removing it was right; "fixing" it would not be. Every PR in this repo
        // carries a changeset, so classifying them as never-docs would force a
        // full build on every PR and the skip would never fire once. This test
        // exists so that change fails loudly instead of quietly neutering the
        // feature.
        expect(isDocsOnlyPath(".changesets/1790043689-ci-something-abcd.md")).toBe(true);
        expect(
            classifyChanges(["docs/specs/SPEC_A.md", ".changesets/1790043689-x.md"]),
        ).toMatchObject({ rust: false, frontend: false, docs_only: true });
    });

    it("rejects the build-controlling root files", () => {
        expect(isDocsOnlyPath("Taskfile.yml")).toBe(false);
        expect(isDocsOnlyPath(".gitattributes")).toBe(false);
        expect(isDocsOnlyPath(".gitignore")).toBe(false);
    });

    it("protects this classifier and its own tests from being skipped", () => {
        // If a PR could edit the skip logic while skipping the build, the
        // system could be disabled in the same change that disables its proof.
        expect(isDocsOnlyPath("scripts/ci-classify-changes.mjs")).toBe(false);
        expect(isDocsOnlyPath("scripts/ci-classify-changes.test.mjs")).toBe(false);
    });
});

describe("isDocsOnlyPath — source files are never documentation", () => {
    it("rejects code, config and lockfiles", () => {
        expect(isDocsOnlyPath("agentmux-srv/src/backend/history/mod.rs")).toBe(false);
        expect(isDocsOnlyPath("frontend/src/App.tsx")).toBe(false);
        expect(isDocsOnlyPath("Cargo.toml")).toBe(false);
        expect(isDocsOnlyPath("Cargo.lock")).toBe(false);
        expect(isDocsOnlyPath("package.json")).toBe(false);
        expect(isDocsOnlyPath("vitest.config.ts")).toBe(false);
    });

    it("does not treat `.md` appearing mid-path as a markdown suffix", () => {
        expect(isDocsOnlyPath("agentmux-srv/src/md.rs")).toBe(false);
        expect(isDocsOnlyPath("frontend/src/markdown/render.ts")).toBe(false);
    });

    it("accepts markdown that sits beside source, having confirmed no build reads it", () => {
        // Checked 2026-09-21: no `include_str!("*.md")` in any crate and no
        // `.md` import in the frontend. If that ever changes, this expectation
        // is the tripwire — and the residual risk is recorded in the spec.
        expect(isDocsOnlyPath("agentmux-srv/README.md")).toBe(true);
    });
});

describe("isDocsOnlyPath — normalization and R2 defaults", () => {
    it("handles the forms git and the API can produce", () => {
        expect(isDocsOnlyPath("docs\\specs\\SPEC_FOO.md")).toBe(true);
        expect(isDocsOnlyPath('"docs/with a space.md"')).toBe(true);
        expect(isDocsOnlyPath("./README.md")).toBe(true);
        expect(isDocsOnlyPath("  docs/spaced.md  ")).toBe(true);
    });

    it("treats empty and malformed input as NOT docs, so the build runs", () => {
        expect(isDocsOnlyPath("")).toBe(false);
        expect(isDocsOnlyPath("   ")).toBe(false);
        expect(isDocsOnlyPath(null)).toBe(false);
        expect(isDocsOnlyPath(undefined)).toBe(false);
    });

    it("treats an unrecognised path as NOT docs (R2, default to running)", () => {
        expect(isDocsOnlyPath("some/brand/new/toolchain.bazel")).toBe(false);
    });
});

describe("classifyChanges — R1, ALL files must be docs, never ANY", () => {
    it("skips both build jobs only when every file is documentation", () => {
        const r = classifyChanges(["docs/specs/A.md", "README.md", "LICENSE"]);
        expect(r).toMatchObject({ rust: false, frontend: false, docs_only: true });
    });

    it("runs everything when a single source file rides along", () => {
        // The case an "ANY file is a doc" filter gets wrong, and the whole
        // reason this is a script rather than a YAML expression.
        const r = classifyChanges(["docs/specs/A.md", "agentmux-srv/src/lib.rs"]);
        expect(r).toMatchObject({ rust: true, frontend: true, docs_only: false });
    });

    it("runs everything when the ride-along is a CI file (R1 + R3)", () => {
        const r = classifyChanges(["README.md", ".github/workflows/ci-pr.yml"]);
        expect(r).toMatchObject({ rust: true, frontend: true, docs_only: false });
    });

    it("names a concrete offending file, so a wrong skip is debuggable from the log", () => {
        const r = classifyChanges(["docs/A.md", "frontend/src/App.tsx"]);
        expect(r.reason).toContain("frontend/src/App.tsx");
    });
});

describe("classifyChanges — R2, uncertainty always resolves to running", () => {
    it("runs everything on an empty list rather than skipping everything", () => {
        // An empty list almost certainly means the API call failed — it does
        // NOT mean "nothing changed, so nothing needs building".
        const r = classifyChanges([]);
        expect(r).toMatchObject({ rust: true, frontend: true, docs_only: false });
    });

    it("runs everything when the list is not an array at all", () => {
        for (const bad of [null, undefined, "docs/A.md", 42, {}]) {
            expect(classifyChanges(bad)).toMatchObject({ rust: true, docs_only: false });
        }
    });

    it("runs everything when the list holds only blanks", () => {
        expect(classifyChanges(["", "   ", "\t"])).toMatchObject({ rust: true, docs_only: false });
    });

    it("runs everything when an entry is not a string", () => {
        // e.g. raw `gh api` JSON objects piped in instead of `.[].filename`.
        expect(classifyChanges([{ filename: "docs/A.md" }])).toMatchObject({
            rust: true,
            docs_only: false,
        });
    });

    it("explains itself when it falls back to running", () => {
        expect(classifyChanges([]).reason).toMatch(/R2/);
    });
});

describe("classifyChanges — the real shapes this repo produces", () => {
    it("a docs-only spec PR skips the build", () => {
        const r = classifyChanges([
            "docs/specs/SPEC_CI_PATH_CONDITIONAL_CHECKS_2026_09_21.md",
            ".changesets/patch-9999.md",
        ]);
        expect(r).toMatchObject({ rust: false, frontend: false, docs_only: true });
    });

    it("a Rust fix with a doc update runs the build", () => {
        const r = classifyChanges([
            "agentmux-srv/src/backend/history/mod.rs",
            "docs/specs/SPEC_CROSS_CHANNEL_AGENT_HISTORY_RESOLUTION_2026_09_21.md",
        ]);
        expect(r).toMatchObject({ rust: true, frontend: true, docs_only: false });
    });

    it("a workflow-only PR runs the build it is editing", () => {
        const r = classifyChanges([".github/workflows/ci-pr.yml"]);
        expect(r).toMatchObject({ rust: true, frontend: true, docs_only: false });
    });
});
