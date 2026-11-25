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

import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import {
    classifyChanges,
    classifyPullFiles,
    classifyQueueEntry,
    isCefInput,
    isDependencyInput,
    isDocsOnlyPath,
    isFrontendOnlyPath,
    isRustOnlyPath,
    versionOnlyChange,
} from "./ci-classify-changes.mjs";

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

    // The job's own step names the test files it runs; a PR that changes any of them, or a script they import, has to
    // run the job on all three OSes, or a Windows-only failure first shows on main (#7447: release-manifest.mjs and its
    // tests changed, the job was skipped on the PR, and the push run to main failed on windows-latest for three hours).
    // Read from the step itself so the two cannot drift apart again.
    it("runs when a test the job runs, or a script that test imports, changes", () => {
        const root = join(dirname(fileURLToPath(import.meta.url)), "..");
        const yml = readFileSync(join(root, ".github/workflows/ci-pr.yml"), "utf8");
        const step = /- name: generator and classifier tests\s+run: npx vitest run ([^\n]+)/.exec(yml);
        expect(step, "the docs-index job's test step is in ci-pr.yml").not.toBeNull();
        const listed = step[1].trim().split(/\s+/);
        expect(listed.length).toBeGreaterThan(3);
        const seen = new Set();
        const walk = (rel) => {
            if (seen.has(rel)) return;
            seen.add(rel);
            const text = readFileSync(join(root, rel), "utf8");
            for (const m of text.matchAll(/(?:import|from)\s*\(?\s*["'](\.{1,2}\/[^"']+)["']/g)) {
                const target = relative(root, resolve(join(root, dirname(rel)), m[1])).replaceAll("\\", "/");
                if (/^scripts\/.+\.m?js$/.test(target) && statSync(join(root, target), { throwIfNoEntry: false })) walk(target);
            }
        };
        listed.forEach(walk);
        expect([...seen].filter((p) => !classifyChanges([p]).docs_index).sort()).toEqual([]);
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
            // The release moves the version in Cargo.toml and Cargo.lock, which the agentmux-cef crate is built from (R10).
            cef: false,
            version_only: true,
        });
        // ... but a release that also changes a crate's code does run it.
        expect(classifyPullFiles([...release(), { filename: "crates/cef/src/ipc.rs", patch: "@@ -1 +1 @@\n-a\n+b" }]).cef).toBe(true);
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

    it("builds when the new or old version isn't a valid semver", () => {
        // Every file agrees, so release consistency passes, but Cargo rejects it.
        for (const [from, to] of [
            ["0.59.15", "definitely not semver"],
            ["0.59.15", "0.59"],
            ["0.59.15", "v0.59.16"],
            ["0.59.15", "0.59.016"],
            ["garbage", "0.59.16"],
        ]) {
            expect(classifyPullFiles(release(from, to))).toMatchObject({ rust: true, version_only: false });
        }
        // Pre-release and build metadata are valid.
        expect(versionOnlyChange(release("0.59.15", "0.60.0-rc.1")).ok).toBe(true);
        expect(versionOnlyChange(release("0.59.15", "0.59.16+build.7")).ok).toBe(true);
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

describe("R8 — a PR that only touches the frontend skips the Rust legs", () => {
    it("runs the frontend legs and no Rust leg when every file is a frontend file no crate reads", () => {
        const r = classifyChanges([
            "frontend/app/statusbar/UpdateStatus.test.tsx",
            "frontend/app/store/app-update.ts",
            "frontend/types/custom.d.ts",
            ".changesets/1794485190-update-in-the-store-9q9m.md",
        ]);
        expect(r).toMatchObject({ rust: false, frontend: true, docs_only: false });
        expect(r.reason).toContain("R8");
    });

    it("runs the Rust legs when one file is read by the crates or the Rust job", () => {
        for (const read of [
            "frontend/types/rpc/Foo.ts",
            "frontend/types/rpc/rpc-schema.json",
            "frontend/app/keybindings/host-keys.json",
            "frontend/app/store/rpc-api/generated-manifest.json",
            "frontend/app/store/widget-singleton.ts",
        ]) {
            expect(classifyChanges(["frontend/app/x.ts", read]), read).toMatchObject({ rust: true, frontend: true });
        }
    });

    it("runs the Rust legs when anything outside frontend/ rides along", () => {
        for (const other of ["crates/srv/src/lib.rs", "package.json", "package-lock.json", ".github/workflows/ci-pr.yml", "scripts/foo.mjs", "Cargo.toml", "tools/x.sh", "settings-template.jsonc"]) {
            expect(classifyChanges(["frontend/app/x.ts", other]), other).toMatchObject({ rust: true });
        }
    });

    it("treats look-alike paths as not frontend (R2, toward running)", () => {
        for (const p of ["Frontend/app/x.ts", "frontend", "frontendx/app/x.ts", "docs/frontend/x.ts", "crates/cef/frontend/x.ts", ""]) {
            expect(isFrontendOnlyPath(p), p).toBe(false);
        }
        expect(isFrontendOnlyPath("./frontend\\app\\x.ts")).toBe(true);
    });

    it("still skips both for documentation alone, and keeps the version-only rule", () => {
        expect(classifyChanges(["docs/specs/SPEC_X.md"])).toMatchObject({ rust: false, frontend: false, docs_only: true });
        const entries = [{ filename: "package.json", patch: '@@ -1,3 +1,3 @@\n-  "version": "0.60.14",\n+  "version": "0.60.15",' }];
        expect(classifyPullFiles(entries)).toMatchObject({ rust: false, frontend: false, version_only: true });
    });

    it("classifies a frontend-only PR from the PR files API too", () => {
        const r = classifyPullFiles([{ filename: "frontend/app/a.ts", patch: "@@ -1 +1 @@\n-a\n+b" }]);
        expect(r).toMatchObject({ rust: false, frontend: true, version_only: false });
    });
});

describe("R8 — a renamed file counts under its old name too", () => {
    it("runs the Rust legs when a file the crates read is moved to a path they do not", () => {
        const r = classifyPullFiles([
            { filename: "frontend/app/store/widget-view.ts", previous_filename: "frontend/app/store/widget-singleton.ts", patch: "@@ -1 +1 @@\n-a\n+b" },
        ]);
        expect(r).toMatchObject({ rust: true, version_only: false });
    });

    it("runs the Rust legs when a Rust file is moved into frontend/", () => {
        const r = classifyPullFiles([{ filename: "frontend/app/x.ts", previous_filename: "crates/srv/src/x.rs", patch: "" }]);
        expect(r).toMatchObject({ rust: true });
    });

    it("keeps the skip for a frontend file moved between frontend paths no crate reads", () => {
        const r = classifyPullFiles([{ filename: "frontend/app/b.ts", previous_filename: "frontend/app/a.ts", patch: "" }]);
        expect(r).toMatchObject({ rust: false, frontend: true });
    });

    it("never takes a rename for a version-only change, and a bad previous name runs everything", () => {
        const patch = '@@ -1,3 +1,3 @@\n-  "version": "0.60.14",\n+  "version": "0.60.15",';
        const r = classifyPullFiles([{ filename: "package.json", patch }, { filename: "docs/b.md", previous_filename: "docs/a.md", patch: "" }]);
        expect(r.version_only).toBe(false);
        expect(classifyPullFiles([{ filename: "frontend/a.ts", previous_filename: 5, patch: "" }])).toMatchObject({ rust: true, frontend: true });
    });
});

describe("R8 — every frontend path a crate names is one the classifier sends to the Rust legs", () => {
    const root = join(dirname(fileURLToPath(import.meta.url)), "..", "crates");
    const rustFiles = (dir) =>
        readdirSync(dir).flatMap((name) => {
            const full = join(dir, name);
            if (name === "target" || name === "node_modules") return [];
            return statSync(full).isDirectory() ? rustFiles(full) : name.endsWith(".rs") ? [full] : [];
        });

    it("finds the paths and lists each one", () => {
        // A path of frontend/ written relative to a crate (`../../frontend/...`): include_str!, a join on
        // CARGO_MANIFEST_DIR, ts-rs's export_to. A new read is a new line here, and the classifier must list it.
        const found = new Map();
        for (const file of rustFiles(root)) {
            const text = readFileSync(file, "utf8");
            for (const m of text.matchAll(/(?:\.\.\/)+(frontend\/[^"'`\s)]*)/g)) found.set(m[1], file);
        }
        expect(found.size).toBeGreaterThanOrEqual(4);
        const unlisted = [...found].filter(([path]) => isFrontendOnlyPath(path)).map(([path, file]) => `${path} (${file})`);
        expect(unlisted, "add these to RUST_READS_FRONTEND in scripts/ci-classify-changes.mjs").toEqual([]);
    });
});

describe("R8 in the merge queue — an entry that changes only documentation and frontend files skips the Rust job", () => {
    const repo = join(dirname(fileURLToPath(import.meta.url)), "..");
    const queue = (...paths) => classifyQueueEntry(paths).rust;

    it("skips the Rust job for a frontend-only entry, and for one that adds a changeset beside it", () => {
        expect(queue("frontend/app/store/app-update.ts", "frontend/app/statusbar/UpdateStatus.test.tsx")).toBe(false);
        expect(queue("frontend/app/a.ts", ".changesets/1795-x.md")).toBe(false);
        expect(queue("docs/specs/SPEC_X.md")).toBe(false);
    });

    it("runs the Rust job for one Rust file, however many frontend files ride along", () => {
        expect(queue("crates/srv/src/lib.rs")).toBe(true);
        expect(queue("frontend/app/a.ts", "crates/srv/src/lib.rs")).toBe(true);
    });

    it("runs the Rust job for each path the Rust side reads from frontend/", () => {
        for (const read of [
            "frontend/types/rpc/Foo.ts",
            "frontend/types/rpc/rpc-schema.json",
            "frontend/app/keybindings/host-keys.json",
            "frontend/app/store/rpc-api/generated-manifest.json",
            "frontend/app/store/widget-singleton.ts",
        ]) {
            expect(queue("frontend/app/a.ts", read), read).toBe(true);
        }
    });

    it("runs the Rust job for a workflow, a script, a manifest and a lock file", () => {
        for (const p of [".github/workflows/ci-pr.yml", "scripts/ci-classify-changes.mjs", "package.json", "Cargo.toml", "Cargo.lock", "Taskfile.yml"]) {
            expect(queue("frontend/app/a.ts", p), p).toBe(true);
        }
    });

    it("runs the Rust job for a path the classifier does not recognise", () => {
        expect(queue("somewhere/new/file.txt")).toBe(true);
        expect(queue("frontend/app/a.ts", "somewhere/new/file.txt")).toBe(true);
    });

    it("runs the Rust job when the list is empty or has an entry that is not a path (R2)", () => {
        expect(queue()).toBe(true);
        expect(classifyQueueEntry(undefined).rust).toBe(true);
        expect(classifyQueueEntry("frontend/a.ts").rust).toBe(true);
        expect(classifyQueueEntry([null, "frontend/a.ts"]).rust).toBe(true);
        expect(classifyQueueEntry(["", "frontend/a.ts"]).rust).toBe(true);
        expect(classifyQueueEntry([5]).rust).toBe(true);
    });

    it("runs the Rust job when a Rust file was moved into frontend/ (a rename lists both names)", () => {
        expect(queue("frontend/app/x.ts", "crates/srv/src/x.rs")).toBe(true);
    });

    it("takes the frontend-only answer from the same rule a pull request uses", () => {
        for (const files of [["frontend/a.ts"], ["frontend/types/rpc/A.ts"], ["crates/x.rs"], ["docs/a.md", "frontend/b.ts"], ["scripts/a.sh"]]) {
            expect(classifyQueueEntry(files).rust, files.join(",")).toBe(classifyChanges(files).rust);
        }
    });

    describe("the command the workflow calls", () => {
        const run = (input) => spawnSync(process.execPath, [join(repo, "scripts", "ci-classify-changes.mjs"), "--queue"], { encoding: "utf8", input });

        it("prints one rust= line and exits 0", () => {
            const skip = run("frontend/app/a.ts\nfrontend/app/b.ts\n");
            expect(skip.status).toBe(0);
            expect(skip.stdout).toBe("rust=false\n");
            const keep = run("frontend/app/a.ts\ncrates/srv/src/lib.rs\n");
            expect(keep.stdout).toBe("rust=true\n");
        });

        it("runs the Rust job on empty input", () => {
            expect(run("").stdout).toBe("rust=true\n");
            expect(run("\n\n").stdout).toBe("rust=true\n");
        });
    });

    describe("the workflow step", () => {
        const yml = readFileSync(join(repo, ".github/workflows/ci-pr.yml"), "utf8");
        const start = yml.indexOf('if [ "${{ github.event_name }}" = "merge_group" ]; then');
        const block = yml.slice(start, yml.indexOf("exit 0", start));

        it("starts from rust=true and lowers it only when the classifier says rust=false", () => {
            expect(start).toBeGreaterThan(0);
            expect(block).toContain("QUEUE_RUST=true");
            expect(block).toMatch(/\[ "\$VERDICT" = "rust=false" \]/);
            expect([...block.matchAll(/QUEUE_RUST=false/g)]).toHaveLength(1);
        });

        it("needs both shas and a clean fetch, diff and classifier run, or it leaves the Rust job on", () => {
            expect(block).toContain('[ -n "$MG_BASE_SHA" ] && [ -n "$MG_HEAD_SHA" ]');
            expect(block).toMatch(/git fetch [^\n]*"\$MG_BASE_SHA" "\$MG_HEAD_SHA"/);
            expect(block).toContain('git diff --name-only --no-renames "$MG_BASE_SHA" "$MG_HEAD_SHA"');
            expect(block).toContain("ci-classify-changes.mjs --queue");
            expect(block).toMatch(/else.*the Rust job runs [(]R2[)]/s);
        });

        it("writes the outputs once, after the decision, and keeps everything else the queue set", () => {
            expect([...block.matchAll(/>> "\$GITHUB_OUTPUT"/g)]).toHaveLength(2);
            expect(block).toContain("rust=%s");
            for (const kept of ["frontend=true", "frontend_build=true", "docs_index=true", "cef=false", "deps=false", "queue=true", 'rust_os=["ubuntu-latest"]']) {
                expect(block, kept).toContain(kept);
            }
        });

        it("reads the shas from the merge_group event", () => {
            expect(yml).toContain("MG_BASE_SHA: ${{ github.event.merge_group.base_sha }}");
            expect(yml).toContain("MG_HEAD_SHA: ${{ github.event.merge_group.head_sha }}");
        });
    });
});

describe("R12 — a documentation file a Rust test reads runs the Rust legs", () => {
    const manifest = "docs/specs/app-api-manifest.json";
    const vectors = "docs/specs/fixtures/redaction-vectors.json";

    it("is not documentation-only", () => {
        expect(isDocsOnlyPath(manifest)).toBe(false);
        expect(isDocsOnlyPath(vectors)).toBe(false);
        expect(isDocsOnlyPath("docs/specs/app-api-manifest.json.bak")).toBe(true);
        expect(isDocsOnlyPath("docs/specs/SPEC_X.md")).toBe(true);
    });

    it("runs the Rust legs for one of them alone, and for one beside documentation", () => {
        for (const files of [[manifest], [vectors], ["docs/specs/SPEC_X.md", manifest], [".changesets/1-x.md", vectors]]) {
            expect(classifyChanges(files), files.join(",")).toMatchObject({ rust: true, docs_only: false });
        }
    });

    it("still skips them for other documentation, and for frontend files no crate reads", () => {
        expect(classifyChanges(["docs/specs/SPEC_X.md", "docs/specs/fixtures/other.json"])).toMatchObject({ rust: false, docs_only: true });
        expect(classifyChanges(["frontend/app/a.ts", "docs/specs/SPEC_X.md"])).toMatchObject({ rust: false, frontend: true });
    });

    it("runs the Rust legs when a listed file moves, from a pull request's file list", () => {
        const r = classifyPullFiles([{ filename: "docs/specs/archive/old.json", previous_filename: manifest, patch: "" }]);
        expect(r).toMatchObject({ rust: true });
    });

    it("runs the Rust job in the merge queue too", () => {
        expect(classifyQueueEntry([manifest]).rust).toBe(true);
        expect(classifyQueueEntry(["docs/specs/SPEC_X.md", "frontend/app/a.ts"]).rust).toBe(false);
    });

    it("lists every file of docs/ that a crate reads from the manifest directory", () => {
        const root = join(dirname(fileURLToPath(import.meta.url)), "..", "crates");
        const rustFiles = (dir) =>
            readdirSync(dir).flatMap((name) => {
                const full = join(dir, name);
                if (name === "target" || name === "node_modules") return [];
                return statSync(full).isDirectory() ? rustFiles(full) : name.endsWith(".rs") ? [full] : [];
            });
        // A file of docs/ named within a few lines of where the test finds the repository root (CARGO_MANIFEST_DIR, or a
        // call of a repo_root() helper). A bare docs path in a test fixture is not a read.
        const found = new Map();
        for (const file of rustFiles(root)) {
            const lines = readFileSync(file, "utf8").split("\n");
            lines.forEach((line, i) => {
                for (const m of line.matchAll(/"(docs\/[^"\s*]*\.(?:json|jsonc|md|txt|toml|ya?ml))"/g)) {
                    const near = lines.slice(Math.max(0, i - 6), i + 1).join("\n");
                    if (/CARGO_MANIFEST_DIR|repo_root\(\)/.test(near)) found.set(m[1], file);
                }
            });
        }
        expect(found.size).toBeGreaterThanOrEqual(2);
        const unlisted = [...found].filter(([path]) => isDocsOnlyPath(path)).map(([path, file]) => `${path} (${file})`);
        expect(unlisted, "add these to RUST_READS_DOCS in scripts/ci-classify-changes.mjs").toEqual([]);
    });
});

describe("R9 — the production frontend build runs unless only Rust files changed", () => {
    it("does not run for Rust sources, their manifests and documentation alone", () => {
        const r = classifyChanges([
            "crates/srv/src/lib.rs",
            "crates/cef/Cargo.toml",
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            ".cargo/config.toml",
            ".config/nextest.toml",
            "docs/specs/SPEC_X.md",
            ".changesets/1-x.md",
        ]);
        // The frontend legs still run for it (vitest reads files of crates/); only the build is left out.
        expect(r).toMatchObject({ rust: true, frontend: true, frontend_build: false });
        expect(r.reason).toMatch(/R9/);
    });

    it("runs when one file rides along that is not a Rust file: what the build reads, and anything unnamed", () => {
        const riders = [
            "frontend/app/app.tsx",
            "frontend/app/app.scss",
            "index.html",
            "vite.config.ts",
            "tsconfig.json",
            "package.json",
            "package-lock.json",
            "scripts/vite-build.sh",
            "scripts/third-party-notices.mjs",
            "schema/color-palette.json",
            "licenses/bundled/x.txt",
            ".github/workflows/ci-pr.yml",
            "Taskfile.yml",
            "some/new/place.txt",
        ];
        for (const rider of riders) {
            const r = classifyChanges(["crates/srv/src/lib.rs", rider]);
            expect(r.frontend_build, rider).toBe(true);
            expect(r.frontend, rider).toBe(true);
        }
    });

    it("runs for a frontend-only PR, and not where the frontend legs do not run at all", () => {
        expect(classifyChanges(["frontend/app/app.tsx"])).toMatchObject({ rust: false, frontend: true, frontend_build: true });
        expect(classifyChanges(["docs/a.md"])).toMatchObject({ frontend: false, frontend_build: false });
        const patch = '@@ -1,3 +1,3 @@\n-  "version": "0.60.14",\n+  "version": "0.60.15",';
        expect(classifyPullFiles([{ filename: "package.json", patch }])).toMatchObject({ version_only: true, frontend_build: false });
    });

    it("R2: runs on an empty, missing or malformed list", () => {
        expect(classifyChanges([]).frontend_build).toBe(true);
        expect(classifyChanges(undefined).frontend_build).toBe(true);
        expect(classifyChanges(["crates/srv/src/lib.rs", 5]).frontend_build).toBe(true);
        expect(classifyPullFiles([{ patch: "" }]).frontend_build).toBe(true);
    });

    it("counts a renamed file under its old name too, and takes look-alike paths for not Rust", () => {
        const moved = classifyPullFiles([{ filename: "crates/srv/src/x.rs", previous_filename: "frontend/app/x.ts", patch: "" }]);
        expect(moved.frontend_build).toBe(true);
        for (const p of ["Crates/srv/x.rs", "crates", "cratesx/a.rs", "frontend/crates/a.rs", "docs/Cargo.toml", "crates-old/a.rs", ""]) {
            expect(isRustOnlyPath(p), p).toBe(false);
        }
        expect(isRustOnlyPath("crates\\srv\\src\\lib.rs")).toBe(true);
    });

    it("holds: nothing the build bundles is read out of crates/", () => {
        // The promise BUILD_NEVER_READS makes. What the build bundles: index.html, the frontend's sources and
        // styles (not its tests, which vitest runs and which do read files of crates/), and the vite config.
        const repo = join(dirname(fileURLToPath(import.meta.url)), "..");
        const bundled = (dir) =>
            readdirSync(dir).flatMap((name) => {
                const full = join(dir, name);
                if (name === "node_modules") return [];
                if (statSync(full).isDirectory()) return bundled(full);
                return /\.(tsx?|s?css|html)$/.test(name) && !/\.test\./.test(name) ? [full] : [];
            });
        const files = [...bundled(join(repo, "frontend")), join(repo, "index.html"), join(repo, "vite.config.ts")];
        expect(files.length).toBeGreaterThan(500);
        const reaching = files.filter((file) => /(?:\.\.\/)+crates\//.test(readFileSync(file, "utf8")));
        expect(reaching, "the build reads these out of crates/: take crates/ out of BUILD_NEVER_READS, or list what is read").toEqual([]);
    });
});

describe("R11 — the known-vulnerability check runs when a lock file, a manifest or the check itself changes", () => {
    it("runs for the lock files, the manifests, the ignore lists and the check's own files", () => {
        for (const p of [
            "Cargo.lock",
            "Cargo.toml",
            "crates/srv/Cargo.toml",
            "crates/srv-core/Cargo.toml",
            "deny.toml",
            "package.json",
            "package-lock.json",
            ".github/npm-audit-ignore.json",
            "scripts/advisory-ignores.mjs",
            "scripts/advisory-ignores.test.mjs",
            "scripts/npm-audit-gate.mjs",
            "scripts/npm-audit-gate.test.mjs",
            ".github/workflows/ci-pr.yml",
        ]) {
            expect(isDependencyInput(p), p).toBe(true);
            expect(classifyChanges([p]).deps, p).toBe(true);
        }
    });

    it("does not run for a source change, a doc or a nested look-alike", () => {
        for (const p of ["crates/srv/src/lib.rs", "frontend/app/App.tsx", "docs/specs/A.md", "crates/srv/src/Cargo.toml", "docs/package.json.md", "xCargo.lock"]) {
            expect(classifyChanges([p]).deps, p).toBe(false);
        }
        expect(classifyChanges(["docs/specs/A.md", ".changesets/1-x.md"]).deps).toBe(false);
    });

    it("runs when a lock file rides along with other files, and on a Windows-style path", () => {
        expect(classifyChanges(["docs/a.md", "crates/srv/src/lib.rs", "Cargo.lock"]).deps).toBe(true);
        expect(classifyChanges(["crates\\srv\\Cargo.toml"]).deps).toBe(true);
    });

    it("runs on anything unrecognised: an empty list, a malformed entry, a bad API answer", () => {
        expect(classifyChanges([]).deps).toBe(true);
        expect(classifyChanges(undefined).deps).toBe(true);
        expect(classifyChanges(["docs/a.md", 7]).deps).toBe(true);
        expect(classifyPullFiles([{ filename: "docs/a.md" }, { nothing: true }]).deps).toBe(true);
    });

    it("runs for a lock change that is not only the release version, and skips a release that moves only the version", () => {
        expect(classifyPullFiles([{ filename: "Cargo.lock", patch: "@@ -1 +1 @@\n-name = \"a\"\n+name = \"b\"" }]).deps).toBe(true);
        const lockOnly = classifyPullFiles([
            { filename: "package.json", patch: '@@ -1,3 +1,3 @@\n {\n-    "version": "0.1.0",\n+    "version": "0.1.1",\n }' },
            { filename: "Cargo.lock", patch: '@@ -1,3 +1,3 @@\n [[package]]\n-version = "0.1.0"\n+version = "0.1.1"' },
        ]);
        expect(lockOnly).toMatchObject({ version_only: true, deps: false });
    });

    it("prints the output the workflow reads", () => {
        const repo = join(dirname(fileURLToPath(import.meta.url)), "..");
        const run = (...files) => spawnSync(process.execPath, [join(repo, "scripts", "ci-classify-changes.mjs"), ...files], { encoding: "utf8" }).stdout;
        expect(run("Cargo.lock")).toContain("deps=true\n");
        expect(run("crates/srv/src/lib.rs")).toContain("deps=false\n");
    });
});

describe("R10 — the agentmux-cef crate's tests run when what it is built from changes", () => {
    const repo = join(dirname(fileURLToPath(import.meta.url)), "..");

    it("runs for the crate, its two path dependencies, the workspace manifests and the files it includes", () => {
        for (const p of [
            "crates/cef/src/browser_api/routes.rs",
            "crates/cef/Cargo.toml",
            "crates/common/src/lib.rs",
            "crates/procstats/src/lib.rs",
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain.toml",
            ".cargo/config.toml",
            "settings-template.jsonc",
            "assets/linux/icons/hicolor/512x512/apps/agentmux.png",
            "frontend/app/keybindings/host-keys.json",
            ".github/workflows/ci-pr.yml",
            "scripts/ci-apt.sh",
            "scripts/ci-classify-changes.mjs",
        ]) {
            expect(classifyChanges([p]).cef, p).toBe(true);
            expect(isCefInput(p), p).toBe(true);
        }
    });

    it("does not run for what the crate is not built from, however close the name", () => {
        for (const p of [
            "crates/srv/src/lib.rs",
            "crates/srv-core/src/lib.rs",
            "crates/launcher/src/main.rs",
            "crates/cefx/src/lib.rs",
            "crates/common-extra/src/lib.rs",
            "frontend/app/app.tsx",
            "docs/specs/SPEC_FOO_2026_11_18.md",
            "scripts/check-audio-creators.mjs",
            "Cargo.toml.bak",
            "assets/macos/x.png",
        ]) {
            expect(classifyChanges([p]).cef, p).toBe(false);
        }
    });

    it("runs when one file of a larger PR qualifies, and for a Windows path", () => {
        expect(classifyChanges(["frontend/app/app.tsx", "docs/a.md", "crates/cef/src/ipc.rs"]).cef).toBe(true);
        expect(classifyChanges(["crates\\cef\\src\\ipc.rs"]).cef).toBe(true);
    });

    it("R2: runs on an empty, missing or malformed list", () => {
        expect(classifyChanges([]).cef).toBe(true);
        expect(classifyChanges(undefined).cef).toBe(true);
        expect(classifyChanges(["frontend/app/app.tsx", 7]).cef).toBe(true);
        expect(classifyPullFiles([{ filename: "docs/a.md" }, { nothing: true }]).cef).toBe(true);
    });

    it("counts a renamed file under its old name too", () => {
        const moved = classifyPullFiles([{ filename: "crates/srv/src/x.rs", previous_filename: "crates/cef/src/x.rs", patch: "" }]);
        expect(moved.cef).toBe(true);
    });

    it("runs for a manifest change that is not only the version (the release case is in R7's tests)", () => {
        expect(classifyPullFiles([{ filename: "Cargo.toml", patch: "@@ -1 +1 @@\n-x = 1\n+x = 2" }]).cef).toBe(true);
    });

    it("prints the output the workflow reads", () => {
        const out = spawnSync(process.execPath, [join(repo, "scripts", "ci-classify-changes.mjs"), "crates/cef/src/ipc.rs"], { encoding: "utf8" });
        expect(out.stdout).toContain("cef=true\n");
        const none = spawnSync(process.execPath, [join(repo, "scripts", "ci-classify-changes.mjs"), "crates/srv/src/lib.rs"], { encoding: "utf8" });
        expect(none.stdout).toContain("cef=false\n");
    });

    it("lists every file outside the crate that its sources include at compile time", () => {
        const crate = join(repo, "crates", "cef");
        const sources = (dir) =>
            readdirSync(dir).flatMap((name) => {
                const full = join(dir, name);
                if (name === "target") return [];
                return statSync(full).isDirectory() ? sources(full) : name.endsWith(".rs") ? [full] : [];
            });
        const outside = new Map();
        for (const file of [...sources(join(crate, "src")), join(crate, "build.rs")]) {
            const text = readFileSync(file, "utf8");
            for (const m of text.matchAll(/include_(?:str|bytes)!\(\s*"([^"]+)"/g)) {
                const target = relative(repo, resolve(dirname(file), m[1])).replaceAll("\\", "/");
                if (!target.startsWith("crates/cef/")) outside.set(target, file);
            }
        }
        // The key table, the settings template and the Linux icon at the time of writing.
        expect(outside.size).toBeGreaterThanOrEqual(3);
        const unlisted = [...outside].filter(([path]) => !isCefInput(path)).map(([path, file]) => `${path} (${file})`);
        expect(unlisted, "add these to CEF_BUILT_FROM in scripts/ci-classify-changes.mjs").toEqual([]);
    });

    it("lists every crate the agentmux-cef crate depends on by path, directly or through another", () => {
        const wanted = new Map();
        const walk = (name) => {
            const toml = readFileSync(join(repo, "crates", name, "Cargo.toml"), "utf8");
            for (const m of toml.matchAll(/path\s*=\s*"\.\.\/([A-Za-z0-9_-]+)"/g)) {
                if (!wanted.has(m[1])) {
                    wanted.set(m[1], name);
                    walk(m[1]);
                }
            }
        };
        walk("cef");
        expect([...wanted.keys()].sort()).toEqual(expect.arrayContaining(["common", "procstats"]));
        const unlisted = [...wanted].filter(([crate]) => !isCefInput(`crates/${crate}/src/lib.rs`)).map(([crate, from]) => `${crate} (from ${from})`);
        expect(unlisted, "add these crates to CEF_BUILT_FROM in scripts/ci-classify-changes.mjs").toEqual([]);
    });
});

describe("R12 — the srv binary's tests also run in one process when a srv crate changes", () => {
    it("runs for a file of either srv crate and for the step's own files", () => {
        for (const p of [
            "crates/srv/src/server/tests.rs",
            "crates/srv/Cargo.toml",
            "crates/srv-core/src/backend/mod.rs",
            ".github/workflows/ci-pr.yml",
            "scripts/ci-srv-single-process.sh",
            "scripts/ci-srv-single-process.test.sh",
            "scripts/ci-classify-changes.mjs",
        ]) {
            expect(classifyChanges([p]).srv, p).toBe(true);
        }
    });

    it("does not run for a PR that touches no srv crate", () => {
        for (const p of [
            "frontend/app/app.tsx",
            "docs/a.md",
            "crates/cef/src/ipc.rs",
            "crates/common/src/lib.rs",
            "crates/mcp/src/main.rs",
            "crates/srvx/src/lib.rs",
            "Cargo.lock",
            "scripts/ci-nextest-run.sh",
        ]) {
            expect(classifyChanges([p]).srv, p).toBe(false);
        }
    });

    it("runs when one changed file among others is a srv file, and when a path uses backslashes", () => {
        expect(classifyChanges(["frontend/app/app.tsx", "docs/a.md", "crates/srv/src/x.rs"]).srv).toBe(true);
        expect(classifyChanges(["crates\\srv\\src\\x.rs"]).srv).toBe(true);
    });

    it("runs on anything unrecognised (R2)", () => {
        expect(classifyChanges([]).srv).toBe(true);
        expect(classifyChanges(undefined).srv).toBe(true);
        expect(classifyChanges(["frontend/app/app.tsx", 7]).srv).toBe(true);
        expect(classifyPullFiles([{ filename: "docs/a.md" }, { nothing: true }]).srv).toBe(true);
    });

    it("counts the old name of a renamed file", () => {
        const moved = classifyPullFiles([{ filename: "crates/cef/src/x.rs", previous_filename: "crates/srv/src/x.rs", patch: "" }]);
        expect(moved.srv).toBe(true);
    });

    it("prints srv= for the workflow", () => {
        const repo = join(dirname(fileURLToPath(import.meta.url)), "..");
        const out = spawnSync(process.execPath, [join(repo, "scripts", "ci-classify-changes.mjs"), "crates/srv/src/x.rs"], { encoding: "utf8" });
        expect(out.stdout).toContain("srv=true\n");
        const none = spawnSync(process.execPath, [join(repo, "scripts", "ci-classify-changes.mjs"), "docs/a.md"], { encoding: "utf8" });
        expect(none.stdout).toContain("srv=false\n");
    });
});
