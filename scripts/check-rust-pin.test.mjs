// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { dockerfileVersion, pinnedChannel, problems, workflowRefs } from "./check-rust-pin.mjs";

const workflow = (ref) => `jobs:\n  a:\n    steps:\n      - uses: dtolnay/rust-toolchain@${ref}\n      # uses: dtolnay/rust-toolchain@stable\n`;

describe("check-rust-pin", () => {
    it("reads the channel and the Dockerfile default", () => {
        expect(pinnedChannel('[toolchain]\nchannel = "1.99.1"\nprofile = "minimal"\n')).toBe("1.99.1");
        expect(pinnedChannel("# channel = \"x\"\n[toolchain]\n")).toBeNull();
        expect(dockerfileVersion("FROM x\nARG RUST_VERSION=1.99.1\nENV A=b\n")).toBe("1.99.1");
        expect(dockerfileVersion("FROM x\n")).toBeNull();
    });

    it("finds the installs in a workflow and skips a commented one", () => {
        expect(workflowRefs(workflow("1.99.1"))).toEqual([{ line: 4, ref: "1.99.1" }]);
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
});
