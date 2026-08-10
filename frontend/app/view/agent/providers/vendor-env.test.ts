// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { resolveVendorEnvOverride } from "./vendor-env";

describe("resolveVendorEnvOverride", () => {
    it("returns null when the agent has no override set", () => {
        expect(resolveVendorEnvOverride(undefined, "ANTHROPIC_BASE_URL")).toBeNull();
        expect(resolveVendorEnvOverride("", "ANTHROPIC_BASE_URL")).toBeNull();
    });

    it("returns null when the provider doesn't declare a base URL env var", () => {
        expect(resolveVendorEnvOverride("https://my-proxy.example.com", undefined)).toBeNull();
    });

    it("returns the [envVar, value] pair when both are present", () => {
        expect(resolveVendorEnvOverride("https://my-proxy.example.com", "ANTHROPIC_BASE_URL")).toEqual([
            "ANTHROPIC_BASE_URL",
            "https://my-proxy.example.com",
        ]);
    });
});
