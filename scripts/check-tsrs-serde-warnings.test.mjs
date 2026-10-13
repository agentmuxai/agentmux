// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// check-tsrs-serde-warnings.mjs: reading ts-rs's warnings out of cargo's output.

import { describe, expect, it } from "vitest";
import { disallowed, unparsedItems } from "./check-tsrs-serde-warnings.mjs";

const output = [
    "    Checking agentmux-srv v0.59.22",
    "warning: failed to parse serde attribute",
    "  |",
    '  | alias = "o_auth_config_dir"',
    "  |",
    "  = note: ts-rs failed to parse this attribute. It will be ignored.",
    "warning: failed to parse serde attribute",
    "  |",
    '  | rename = "x", with = "custom"',
    "  |",
    "warning: unused import: `Foo`",
].join("\n");

describe("unparsedItems", () => {
    it("reads the attribute item shown under each warning", () => {
        expect(unparsedItems(output)).toEqual(['alias = "o_auth_config_dir"', 'rename = "x", with = "custom"']);
    });

    it("finds nothing in a clean build", () => {
        expect(unparsedItems("    Finished `dev` profile in 0.57s")).toEqual([]);
    });
});

describe("disallowed", () => {
    it("lets a deserialize-only alias through, and nothing else", () => {
        expect(disallowed(unparsedItems(output))).toEqual(['rename = "x", with = "custom"']);
    });
});
