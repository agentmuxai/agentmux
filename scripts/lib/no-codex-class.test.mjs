// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    cargoLockVersionOnly,
    cargoTomlVersionOnly,
    isNoCodePr,
    noCodePath,
    packageJsonVersionOnly,
    packageLockVersionOnly,
    PR_FILE_CAP,
} from "./no-codex-class.mjs";

const pkg = (version, extra = {}) => JSON.stringify({ name: "agentmux", version, dependencies: { left: "1.0.0" }, ...extra }, null, 4);

const lock = (rootVersion, { left = "1.0.0", workspace = rootVersion } = {}) =>
    JSON.stringify(
        {
            name: "agentmux",
            version: rootVersion,
            lockfileVersion: 3,
            packages: {
                "": { name: "agentmux", version: rootVersion, dependencies: { left: "^1.0.0" } },
                "packages/sdk": { name: "sdk", version: workspace },
                "node_modules/left": { version: left, resolved: "https://registry.npmjs.org/left/-/left-1.tgz", integrity: "sha512-x" },
                "node_modules/sdk": { resolved: "packages/sdk", link: true },
            },
        },
        null,
        2,
    );

const cargoToml = (version, dep = '"1.0"') =>
    `[workspace]\nmembers = ["crates/a"]\n\n[workspace.package]\nversion = "${version}"\nedition = "2021"\n\n[workspace.dependencies]\nserde = { version = ${dep} }\n`;

const cargoLock = (ours, serde = "1.0.1") =>
    `version = 4\n\n[[package]]\nname = "agentmux-srv"\nversion = "${ours}"\ndependencies = [\n "serde",\n]\n\n` +
    `[[package]]\nname = "serde"\nversion = "${serde}"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\nchecksum = "abc"\n`;

describe("noCodePath", () => {
    it.each([
        "docs/specs/SPEC_X_2026_10_29.md",
        "docs/reports/REPORT_X.md",
        "docs/guide/intro.mdx",
        "specs/design.md",
        "docs-site/pages/start.mdx",
        ".changesets/1793-a-fix-ab12.md",
        ".changeset/x.md",
        "README.md",
        "CHANGELOG.md",
        "LICENSE",
        "VERSION_HISTORY.md",
        "docs/VERSION_HISTORY.md",
        "frontend/assets/logo.png",
        "docs/screens/pane.jpg",
        "media/demo.webm",
        "media/note.wav",
        "fonts/inter.woff2",
    ])("%s has no code", (path) => expect(noCodePath(path)).toBe(true));

    it.each([
        "src/main.rs",
        "docs/sample.json",
        "docs/build.sh",
        "docs/CLAUDE.md",
        "CLAUDE.md",
        "AGENTS.md",
        "docs/skills/SKILL.md",
        "prompts/review.md",
        "scripts/notes.md",
        ".github/workflows/ci.yml",
        ".github/ISSUE_TEMPLATE/bug.md",
        ".claude/skills/release/SKILL.md",
        "tools/README.md",
        "crates/srv/README.md",
        "notes.md",
        "icons/logo.svg",
        "package.json",
        "",
    ])("%s is code", (path) => expect(noCodePath(path)).toBe(false));
});

describe("package.json", () => {
    it("accepts a changed version", () => expect(packageJsonVersionOnly(pkg("0.60.6"), pkg("0.60.7"))).toBe(true));
    it("refuses a changed dependency", () => {
        expect(packageJsonVersionOnly(pkg("0.60.6"), pkg("0.60.7", { dependencies: { left: "2.0.0" } }))).toBe(false);
    });
    it("refuses a new script beside the version", () => {
        expect(packageJsonVersionOnly(pkg("0.60.6"), pkg("0.60.7", { scripts: { postinstall: "curl x | sh" } }))).toBe(false);
    });
    it("refuses a version that is not a version", () => expect(packageJsonVersionOnly(pkg("0.60.6"), pkg("latest"))).toBe(false));
    it("refuses an unchanged version and broken JSON", () => {
        expect(packageJsonVersionOnly(pkg("0.60.6"), pkg("0.60.6"))).toBe(false);
        expect(packageJsonVersionOnly(pkg("0.60.6"), "{")).toBe(false);
    });
});

describe("package-lock.json", () => {
    it("accepts the root and a workspace package at a new version", () => {
        expect(packageLockVersionOnly(lock("0.60.6"), lock("0.60.7"))).toBe(true);
    });
    it("refuses a registry package at a new version", () => {
        expect(packageLockVersionOnly(lock("0.60.6"), lock("0.60.7", { left: "1.0.1" }))).toBe(false);
    });
    it("refuses a dependency under node_modules even when it has no resolved or integrity", () => {
        const stripped = (v) => {
            const l = JSON.parse(lock("0.60.7"));
            l.packages["node_modules/left"] = { version: v };
            return JSON.stringify(l);
        };
        expect(packageLockVersionOnly(stripped("1.0.0"), stripped("1.0.1"))).toBe(false);
        const nested = (v) => {
            const l = JSON.parse(lock("0.60.7"));
            l.packages["packages/sdk/node_modules/left"] = { version: v };
            return JSON.stringify(l);
        };
        expect(packageLockVersionOnly(nested("1.0.0"), nested("1.0.1"))).toBe(false);
    });
    it("refuses a new dependency entry", () => {
        const next = JSON.parse(lock("0.60.7"));
        next.packages["node_modules/evil"] = { version: "1.0.0", resolved: "https://example.test/evil.tgz", integrity: "sha512-y" };
        expect(packageLockVersionOnly(lock("0.60.6"), JSON.stringify(next))).toBe(false);
    });
});

describe("Cargo.toml", () => {
    it("accepts the package version", () => expect(cargoTomlVersionOnly(cargoToml("0.60.6"), cargoToml("0.60.7"))).toBe(true));
    it("refuses a dependency's version, even as a version = line", () => {
        expect(cargoTomlVersionOnly(cargoToml("0.60.6"), cargoToml("0.60.6", '"2.0"'))).toBe(false);
        const base = '[dependencies.serde]\nversion = "1.0.0"\n';
        expect(cargoTomlVersionOnly(base, '[dependencies.serde]\nversion = "1.0.1"\n')).toBe(false);
    });
    it("refuses an added line", () => {
        expect(cargoTomlVersionOnly(cargoToml("0.60.6"), `${cargoToml("0.60.7")}extra = 1\n`)).toBe(false);
    });
    it("reads a table header that has a trailing comment, so a dependency's version is still refused", () => {
        const base = '[package]\nname = "a"\nversion = "0.1.0"\n\n[dependencies.serde] # pinned\nversion = "1.0.0"\n';
        expect(cargoTomlVersionOnly(base, base.replace('"1.0.0"', '"1.0.1"'))).toBe(false);
        expect(cargoTomlVersionOnly(base, base.replace('"0.1.0"', '"0.1.1"'))).toBe(true);
        const odd = '[package]\nversion = "0.1.0"\n\n[dependencies."x[1]"]\nversion = "1.0.0"\n';
        expect(cargoTomlVersionOnly(odd, odd.replace('"1.0.0"', '"1.0.1"'))).toBe(false);
    });
});

describe("Cargo.lock", () => {
    it("accepts our own crate at a new version", () => expect(cargoLockVersionOnly(cargoLock("0.60.6"), cargoLock("0.60.7"))).toBe(true));
    it("refuses a registry crate at a new version", () => {
        expect(cargoLockVersionOnly(cargoLock("0.60.6"), cargoLock("0.60.6", "1.0.2"))).toBe(false);
    });
    it("refuses a changed checksum or source", () => {
        expect(cargoLockVersionOnly(cargoLock("0.60.6"), cargoLock("0.60.7").replace('checksum = "abc"', 'checksum = "def"'))).toBe(false);
    });
});

// The pulls files API shape.
const file = (filename, status = "modified", extra = {}) => ({ filename, status, ...extra });

/** readText over a map {path: [base, head]}. */
const reader = (texts) => async (path, side) => {
    const pair = texts[path];
    return pair ? pair[side === "base" ? 0 : 1] : null;
};

const release = {
    "package.json": [pkg("0.60.6"), pkg("0.60.7")],
    "package-lock.json": [lock("0.60.6"), lock("0.60.7")],
    "Cargo.toml": [cargoToml("0.60.6"), cargoToml("0.60.7")],
    "Cargo.lock": [cargoLock("0.60.6"), cargoLock("0.60.7")],
};

describe("isNoCodePr", () => {
    it("a release PR: version lines, VERSION_HISTORY.md and deleted changesets", async () => {
        const files = [
            file("package.json"),
            file("package-lock.json"),
            file("Cargo.toml"),
            file("Cargo.lock"),
            file("VERSION_HISTORY.md"),
            file(".changesets/1793-a-fix-ab12.md", "removed"),
            file(".changesets/1794-b-fix-cd34.md", "removed"),
        ];
        expect(await isNoCodePr({ files, readText: reader(release) })).toMatchObject({ ok: true });
    });

    it("a docs-only PR", async () => {
        const files = [file("docs/specs/SPEC_X.md", "added"), file("docs/reports/R.md"), file(".changesets/x-ab12.md", "added")];
        expect(await isNoCodePr({ files, readText: reader({}) })).toMatchObject({ ok: true });
    });

    it("an assets-only PR", async () => {
        const files = [file("frontend/assets/logo.png", "added"), file("media/demo.webm", "added"), file("fonts/inter.woff2", "modified")];
        expect(await isNoCodePr({ files, readText: reader({}) })).toMatchObject({ ok: true });
    });

    it("docs and assets together are still no code", async () => {
        const files = [file("docs/guide/a.md"), file("docs/screens/a.png", "added")];
        expect(await isNoCodePr({ files, readText: reader({}) })).toMatchObject({ ok: true });
    });

    it("a mixed PR: one line of code in a release PR gets Codex", async () => {
        const files = [file("package.json"), file("VERSION_HISTORY.md"), file("crates/srv/src/lib.rs")];
        const r = await isNoCodePr({ files, readText: reader(release) });
        expect(r).toEqual({ ok: false, why: "crates/srv/src/lib.rs is code" });
    });

    it("a docs PR that touches a workflow, a script or the review gate gets Codex", async () => {
        for (const code of [".github/workflows/ci.yml", "scripts/ci-codex-review-gate.mjs", "config/repos.json", "Taskfile.yml"]) {
            const r = await isNoCodePr({ files: [file("docs/a.md"), file(code)], readText: reader({}) });
            expect(r.ok, code).toBe(false);
        }
    });

    it("a dependency change in a manifest or lockfile gets Codex", async () => {
        const dep = { ...release, "package.json": [pkg("0.60.6"), pkg("0.60.7", { dependencies: { left: "2.0.0" } })] };
        expect((await isNoCodePr({ files: [file("package.json")], readText: reader(dep) })).ok).toBe(false);
        const lockDep = { "Cargo.lock": [cargoLock("0.60.6"), cargoLock("0.60.6", "1.0.2")] };
        expect((await isNoCodePr({ files: [file("Cargo.lock")], readText: reader(lockDep) })).ok).toBe(false);
    });

    it("a version file that can't be read gets Codex", async () => {
        expect(await isNoCodePr({ files: [file("package.json")], readText: reader({}) })).toMatchObject({ ok: false });
    });

    it("a version file that is added, removed or renamed gets Codex", async () => {
        for (const f of [file("package.json", "added"), file("package.json", "removed"), file("package.json", "renamed", { previous_filename: "old/package.json" })]) {
            expect((await isNoCodePr({ files: [f], readText: reader(release) })).ok).toBe(false);
        }
    });

    it("code moved into docs is still a change to code", async () => {
        const f = file("docs/notes.md", "renamed", { previous_filename: "src/main.rs" });
        expect((await isNoCodePr({ files: [f], readText: reader({}) })).ok).toBe(false);
    });

    it("an empty, missing or cut-short file list gets Codex", async () => {
        expect((await isNoCodePr({ files: [], readText: reader({}) })).ok).toBe(false);
        expect((await isNoCodePr({ files: null, readText: reader({}) })).ok).toBe(false);
        const many = Array.from({ length: PR_FILE_CAP }, (_, i) => file(`docs/a${i}.md`));
        expect((await isNoCodePr({ files: many, readText: reader({}) })).ok).toBe(false);
    });
});
