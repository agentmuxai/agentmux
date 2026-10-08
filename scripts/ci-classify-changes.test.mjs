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
import { classifyChanges, classifyPullFiles, isDocsOnlyPath, versionOnlyChange } from "./ci-classify-changes.mjs";

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
        expect(isDocsOnlyPath("crates/srv/src/backend/history/mod.rs")).toBe(false);
        expect(isDocsOnlyPath("frontend/src/App.tsx")).toBe(false);
        expect(isDocsOnlyPath("Cargo.toml")).toBe(false);
        expect(isDocsOnlyPath("Cargo.lock")).toBe(false);
        expect(isDocsOnlyPath("package.json")).toBe(false);
        expect(isDocsOnlyPath("vitest.config.ts")).toBe(false);
    });

    it("does not treat `.md` appearing mid-path as a markdown suffix", () => {
        expect(isDocsOnlyPath("crates/srv/src/md.rs")).toBe(false);
        expect(isDocsOnlyPath("frontend/src/markdown/render.ts")).toBe(false);
    });

    it("accepts markdown that sits beside source, having confirmed no build reads it", () => {
        // Checked 2026-09-21: no `include_str!("*.md")` in any crate and no
        // `.md` import in the frontend. If that ever changes, this expectation
        // is the tripwire — and the residual risk is recorded in the spec.
        expect(isDocsOnlyPath("crates/srv/README.md")).toBe(true);
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
        const r = classifyChanges(["docs/specs/A.md", "crates/srv/src/lib.rs"]);
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
            "crates/srv/src/backend/history/mod.rs",
            "docs/specs/SPEC_CROSS_CHANNEL_AGENT_HISTORY_RESOLUTION_2026_09_21.md",
        ]);
        expect(r).toMatchObject({ rust: true, frontend: true, docs_only: false });
    });

    it("a workflow-only PR runs the build it is editing", () => {
        const r = classifyChanges([".github/workflows/ci-pr.yml"]);
        expect(r).toMatchObject({ rust: true, frontend: true, docs_only: false });
    });
});

describe("classifyChanges — docs_index, the cross-platform specs-index job", () => {
    // docs/specs/SPEC_DOCS_INDEX_GENERATOR_NODE_PORT_2026_09_23.md §7. Skipping
    // this job is only safe when nothing that can make the generator's output
    // differ by OS has changed; every such path must turn it on.
    it("runs when the generator, its tests or fixtures change", () => {
        for (const p of [
            "scripts/gen-docs-index.mjs",
            "scripts/gen-docs-index.sh",
            "scripts/gen-docs-index.test.mjs",
            "scripts/test-fixtures/gen-docs-index/statuses.golden",
            "scripts/test-fixtures/gen-docs-index/fixtures.mjs",
            "scripts/cli-probe/probe.mjs",
            "scripts/cli-probe/fake-anthropic.mjs",
        ]) {
            expect(classifyChanges([p]).docs_index, p).toBe(true);
        }
    });

    it("runs when the job, its classifier or its runtime changes", () => {
        for (const p of [
            ".github/workflows/ci-pr.yml",
            "scripts/ci-classify-changes.mjs",
            "package.json",
            "package-lock.json",
            "vitest.config.ts",
            "vite.config.ts",
            // Attributes change the bytes the generator reads (Codex P2, #3590).
            ".gitattributes",
        ]) {
            expect(classifyChanges([p]).docs_index, p).toBe(true);
        }
    });

    it("runs when any one file in a larger PR qualifies", () => {
        expect(classifyChanges(["docs/specs/A.md", "frontend/app/x.ts", "scripts/gen-docs-index.mjs"]).docs_index).toBe(
            true
        );
    });

    it("does not run for spec-only or unrelated changes (the Linux docs job still asserts specs)", () => {
        expect(classifyChanges(["docs/specs/SPEC_X_2026_09_23.md", "docs/specs/INDEX.md"]).docs_index).toBe(false);
        expect(classifyChanges(["crates/srv/src/lib.rs", "frontend/app/App.tsx"]).docs_index).toBe(false);
        expect(classifyChanges(["scripts/check-doc-status.sh"]).docs_index).toBe(false);
    });

    it("R2: runs on an empty, missing or malformed list", () => {
        expect(classifyChanges([]).docs_index).toBe(true);
        expect(classifyChanges(null).docs_index).toBe(true);
        expect(classifyChanges(["", " "]).docs_index).toBe(true);
        expect(classifyChanges([{ filename: "docs/A.md" }]).docs_index).toBe(true);
        expect(classifyChanges(["docs/A.md", 42]).docs_index).toBe(true);
    });

    it("tolerates git quoting and Windows separators, like the other outputs", () => {
        expect(classifyChanges(['"scripts/gen-docs-index.mjs"']).docs_index).toBe(true);
        expect(classifyChanges(["scripts\\gen-docs-index.mjs"]).docs_index).toBe(true);
    });
});

// R7: a release PR (scripts/release.sh) moves the version in four manifests and
// adds changelog lines; it needs none of the build jobs. The patches below have
// the shape of the PR files API's `patch` for a real release (#4493).
describe("R7 — version-only release PRs", () => {
    const toml = (from, to) => `@@ -9,7 +9,7 @@ members = [\n [workspace.package]\n-version = "${from}"\n+version = "${to}"\n edition = "2021"`;
    const lock = (from, to) =>
        `@@ -32,7 +32,7 @@ dependencies = [\n \n [[package]]\n name = "agentmux-srv"\n-version = "${from}"\n+version = "${to}"\n dependencies = [\n@@ -53,7 +53,7 @@\n name = "agentmux-cef"\n-version = "${from}"\n+version = "${to}"`;
    const pkg = (from, to) => `@@ -1,6 +1,6 @@\n {\n     "name": "agentmux",\n-    "version": "${from}",\n+    "version": "${to}",\n     "private": true,`;
    const lockJson = (from, to) =>
        `@@ -1,12 +1,12 @@\n {\n     "name": "agentmux",\n-    "version": "${from}",\n+    "version": "${to}",\n@@ -8,7 +8,7 @@\n         "": {\n-            "version": "${from}",\n+            "version": "${to}",`;
    const release = (from = "0.59.15", to = "0.59.16") => [
        { filename: "Cargo.toml", patch: toml(from, to) },
        { filename: "Cargo.lock", patch: lock(from, to) },
        { filename: "package.json", patch: pkg(from, to) },
        { filename: "package-lock.json", patch: lockJson(from, to) },
        { filename: "VERSION_HISTORY.md", patch: "@@ -1,3 +1,8 @@\n+## 0.59.16\n+- a fix" },
        { filename: ".changesets/1791-fix-abcd.md", patch: "@@ -1,4 +0,0 @@\n----\n-type: patch\n-----\n-a fix" },
    ];

    it("skips the build jobs for a release PR", () => {
        expect(versionOnlyChange(release())).toEqual({ ok: true, from: "0.59.15", to: "0.59.16" });
        expect(classifyPullFiles(release())).toMatchObject({
            rust: false,
            frontend: false,
            docs_index: false,
            docs_only: false,
            version_only: true,
        });
    });

    it("runs everything when the release also changes code or CI", () => {
        for (const extra of ["crates/srv/src/main.rs", "frontend/app/app.tsx", ".github/workflows/ci-pr.yml", "scripts/release.sh"]) {
            const r = classifyPullFiles([...release(), { filename: extra, patch: "@@ -1 +1 @@\n-a\n+b" }]);
            expect(r).toMatchObject({ rust: true, frontend: true, version_only: false });
        }
    });

    it("runs everything when a manifest changes more than its version", () => {
        // A dependency bump next to the version bump: a second version pair.
        const dep = release();
        dep[1] = { filename: "Cargo.lock", patch: lock("0.59.15", "0.59.16") + '\n name = "serde"\n-version = "1.0.210"\n+version = "1.0.211"' };
        expect(classifyPullFiles(dep)).toMatchObject({ rust: true, version_only: false });
        // A lockfile checksum, or a new dependency line.
        const sum = release();
        sum[1] = { filename: "Cargo.lock", patch: lock("0.59.15", "0.59.16") + '\n-checksum = "aa"\n+checksum = "bb"' };
        expect(classifyPullFiles(sum).version_only).toBe(false);
        const added = release();
        added[2] = { filename: "package.json", patch: pkg("0.59.15", "0.59.16") + '\n+        "left-pad": "^1.3.0",' };
        expect(classifyPullFiles(added).version_only).toBe(false);
    });

    it("needs the release version itself to move, in package.json", () => {
        // A Dependabot lockfile bump: version lines, but not the release version.
        const lockOnly = [{ filename: "package-lock.json", patch: lockJson("7.1.4", "7.1.5") }];
        expect(classifyPullFiles(lockOnly)).toMatchObject({ frontend: true, version_only: false });
        // Two different moves.
        const mixed = release();
        mixed[0] = { filename: "Cargo.toml", patch: toml("0.59.14", "0.59.16") };
        expect(classifyPullFiles(mixed).version_only).toBe(false);
        // No move at all.
        expect(versionOnlyChange(release("0.59.15", "0.59.15")).ok).toBe(false);
    });

    it("runs everything when a manifest's diff is missing", () => {
        // The API omits `patch` for a very large or binary diff.
        const noPatch = release();
        delete noPatch[3].patch;
        expect(classifyPullFiles(noPatch)).toMatchObject({ rust: true, version_only: false });
    });

    it("only counts the root manifests", () => {
        // A crate's own Cargo.toml inherits the workspace version, so a change there is code.
        const crate = [...release(), { filename: "crates/srv/Cargo.toml", patch: toml("0.1.0", "0.2.0") }];
        expect(classifyPullFiles(crate)).toMatchObject({ rust: true, version_only: false });
    });

    it("treats malformed API input as run-everything (R2)", () => {
        for (const bad of [null, [], [{}], [{ filename: 42 }], ["Cargo.toml"]]) {
            expect(classifyPullFiles(bad)).toMatchObject({ rust: true, frontend: true, version_only: false });
        }
    });

    it("still classifies a docs-only PR by its paths", () => {
        expect(classifyPullFiles([{ filename: "docs/A.md", patch: "@@ -1 +1 @@\n-a\n+b" }])).toMatchObject({
            rust: false,
            docs_only: true,
            version_only: false,
        });
    });
});
