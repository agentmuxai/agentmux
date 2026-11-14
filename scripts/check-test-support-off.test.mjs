// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";
import { commandFiles, commandProblems, resolvedProblems } from "./check-test-support-off.mjs";

const tree = (srvFeatures, extra = "") =>
    `agentmux-common v0.60.27 (/w/crates/common) features=[ts,tls-provider]\n` +
    `agentmux-srv v0.60.27 (/w/crates/srv) features=[${srvFeatures}]\n` +
    `serde v1.0.0 features=[derive]\n` +
    `ctor v1.0.0 features=[]\n${extra}`;

describe("check-test-support-off: the commands", () => {
    it("passes a command that names neither", () => {
        expect(commandProblems("run: cargo build --release --workspace\nrun: cargo nextest run --profile ci\n")).toEqual([]);
    });

    it("finds --all-features and the feature by name, with their lines", () => {
        const text = "a\nrun: cargo build --all-features\nrun: cargo test -p agentmux-srv --features test-support\nb: cargo build -F test-support\n";
        expect(commandProblems(text)).toEqual([
            { line: 2, what: "--all-features" },
            { line: 3, what: "test-support" },
            { line: 4, what: "test-support" },
        ]);
    });

    it("skips comment lines and names that only contain it", () => {
        expect(commandProblems("# cargo build --all-features\n// --features test-support\n")).toEqual([]);
        expect(commandProblems("use crate::test_support;\nmy-test-support-lib\ntest-supports\n")).toEqual([]);
    });
});

describe("check-test-support-off: the files", () => {
    it("reads workflows, Taskfile.yml, scripts, docker and .cargo, and skips this check and tests", () => {
        const root = mkdtempSync(join(tmpdir(), "tso-"));
        try {
            const put = (rel) => {
                const full = join(root, rel);
                mkdirSync(join(full, ".."), { recursive: true });
                writeFileSync(full, "x\n");
            };
            for (const rel of [
                ".github/workflows/ci.yml",
                ".github/actions/x/action.yaml",
                "Taskfile.yml",
                "scripts/build.sh",
                "scripts/sub/pack.ps1",
                "docker/Dockerfile.agent",
                ".cargo/config.toml",
                "scripts/check-test-support-off.mjs",
                "scripts/check-test-support-off.test.mjs",
                "scripts/other.test.mjs",
                "scripts/notes.md",
                "crates/srv/Cargo.toml",
            ]) {
                put(rel);
            }
            const found = commandFiles(root).map((f) => relative(root, f).replaceAll("\\", "/")).sort();
            expect(found).toEqual([
                ".cargo/config.toml",
                ".github/actions/x/action.yaml",
                ".github/workflows/ci.yml",
                "Taskfile.yml",
                "docker/Dockerfile.agent",
                "scripts/build.sh",
                "scripts/sub/pack.ps1",
            ]);
        } finally {
            rmSync(root, { recursive: true, force: true });
        }
    });
});

describe("check-test-support-off: the resolve", () => {
    it("passes when no package here has the feature on", () => {
        expect(resolvedProblems(tree(""))).toEqual([]);
    });

    it("names a package here that has it on, once", () => {
        expect(resolvedProblems(tree("test-support"))).toEqual(["agentmux-srv resolves with test-support on in a normal build"]);
        expect(resolvedProblems(tree("a, test-support"))).toEqual(["agentmux-srv resolves with test-support on in a normal build"]);
        expect(resolvedProblems(tree("test-support") + tree("test-support"))).toHaveLength(1);
    });

    it("ignores a registry crate with a feature of the same name, and a feature that only contains it", () => {
        expect(resolvedProblems(tree("", "other v1.0.0 (registry+https://x) features=[test-support]\n"))).toEqual([]);
        expect(resolvedProblems(tree("test-support-extra"))).toEqual([]);
    });

    it("fails an empty or unreadable answer instead of passing it", () => {
        expect(resolvedProblems("")).toEqual(["the resolve did not list agentmux-srv: the check read nothing"]);
        expect(resolvedProblems("error: no\n")).toHaveLength(1);
    });
});
