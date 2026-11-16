// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";
import { dockerfileVersion, installerFiles, pinnedChannel, pinnedComponents, problems, workflowRefs } from "./check-rust-pin.mjs";

const workflow = (ref, components = "clippy, rustfmt") =>
    `jobs:\n  a:\n    steps:\n      - uses: dtolnay/rust-toolchain@${ref}\n${components ? `        with:\n          components: ${components}\n` : ""}      # uses: dtolnay/rust-toolchain@stable\n`;

describe("check-rust-pin", () => {
    it("reads the channel and the Dockerfile default", () => {
        expect(pinnedChannel('[toolchain]\nchannel = "1.99.1"\nprofile = "minimal"\n')).toBe("1.99.1");
        expect(pinnedChannel("# channel = \"x\"\n[toolchain]\n")).toBeNull();
        expect(dockerfileVersion("FROM x\nARG RUST_VERSION=1.99.1\nENV A=b\n")).toBe("1.99.1");
        expect(dockerfileVersion("FROM x\n")).toBeNull();
    });

    it("finds the installs in a workflow and skips a commented one", () => {
        expect(workflowRefs(workflow("1.99.1"))).toEqual([{ line: 4, ref: "1.99.1", components: ["clippy", "rustfmt"] }]);
    });

    it("passes when everything names the pinned version", () => {
        expect(problems("1.99.1", { "a.yml": workflow("1.99.1") }, "ARG RUST_VERSION=1.99.1\n")).toEqual([]);
    });

    it("names each place that disagrees, and refuses a moving channel", () => {
        const found = problems("1.99.1", { "a.yml": workflow("stable"), "b.yml": workflow("1.98.1") }, "ARG RUST_VERSION=1.98.1\n");
        expect(found).toEqual([
            "a.yml:4: dtolnay/rust-toolchain@stable should be @1.99.1",
            "b.yml:4: dtolnay/rust-toolchain@1.98.1 should be @1.99.1",
            "docker/Dockerfile.agent-developer: ARG RUST_VERSION=1.98.1 should be 1.99.1",
        ]);
        expect(problems("stable", {}, "")[0]).toMatch(/full version/);
        expect(problems(null, {}, "")[0]).toMatch(/full version/);
    });

    it("reads the components rust-toolchain.toml lists, and the ones a step installs", () => {
        expect(pinnedComponents('[toolchain]\nchannel = "1.99.1"\ncomponents = ["rustfmt", "clippy"]\n')).toEqual(["clippy", "rustfmt"]);
        expect(pinnedComponents('[toolchain]\nchannel = "1.99.1"\n')).toEqual([]);
        expect(workflowRefs(workflow("1.99.1", "rustfmt clippy"))[0].components).toEqual(["clippy", "rustfmt"]);
        expect(workflowRefs(workflow("1.99.1", ""))[0].components).toEqual([]);
        // A later step's components are not this step's.
        const two = `steps:\n  - uses: dtolnay/rust-toolchain@1.99.1\n  - uses: other/x@1\n    with:\n      components: clippy\n`;
        expect(workflowRefs(two)[0].components).toEqual([]);
    });

    it("names an install that leaves a listed component to be added in the middle of the build", () => {
        const list = ["clippy", "rustfmt"];
        const ok = { "a.yml": workflow("1.99.1") };
        expect(problems("1.99.1", ok, "ARG RUST_VERSION=1.99.1\n", list)).toEqual([]);
        expect(problems("1.99.1", { "a.yml": workflow("1.99.1", "") }, "ARG RUST_VERSION=1.99.1\n", list)).toEqual([
            "a.yml:4: dtolnay/rust-toolchain@1.99.1 should install the components rust-toolchain.toml lists (with: components: clippy, rustfmt), not none",
        ]);
        expect(problems("1.99.1", { "a.yml": workflow("1.99.1", "clippy") }, "ARG RUST_VERSION=1.99.1\n", list)[0]).toMatch(/not clippy$/);
        // Without a components list the check is the version only, as before.
        expect(problems("1.99.1", { "a.yml": workflow("1.99.1", "") }, "ARG RUST_VERSION=1.99.1\n")).toEqual([]);
    });

    it("looks in .yml and .yaml workflows and in composite actions, and nowhere else", () => {
        const root = mkdtempSync(join(tmpdir(), "rust-pin-"));
        try {
            for (const d of ["workflows", "actions/x", "other"]) mkdirSync(join(root, d), { recursive: true });
            for (const f of ["workflows/a.yml", "workflows/b.yaml", "workflows/notes.md", "actions/x/action.yml", "actions/x/data.yml", "other/c.yml"]) writeFileSync(join(root, f), "");
            expect(installerFiles(root).map((f) => relative(root, f).replaceAll("\\", "/")).sort()).toEqual(["actions/x/action.yml", "workflows/a.yml", "workflows/b.yaml"]);
        } finally {
            rmSync(root, { recursive: true, force: true });
        }
    });
});
