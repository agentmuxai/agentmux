// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { resolveEffectiveVendor } from "./catalog";

describe("resolveEffectiveVendor", () => {
    it("returns the provider's default vendor when there's no base URL override", () => {
        expect(resolveEffectiveVendor("claude", undefined)).toBe("anthropic");
        expect(resolveEffectiveVendor("claude", "")).toBe("anthropic");
        expect(resolveEffectiveVendor("claude", null)).toBe("anthropic");
        expect(resolveEffectiveVendor("codex", undefined)).toBe("openai");
        expect(resolveEffectiveVendor("gemini", undefined)).toBe("google");
    });

    it("returns the FIRST supportedVendors entry for a multi-vendor harness", () => {
        expect(resolveEffectiveVendor("openclaw", undefined)).toBe("openai");
        expect(resolveEffectiveVendor("muxcode", undefined)).toBe("ollama");
    });

    it("returns \"custom\" whenever a base URL override is set, regardless of provider", () => {
        expect(resolveEffectiveVendor("claude", "https://my-proxy.example.com")).toBe("custom");
        expect(resolveEffectiveVendor("claude", "  https://my-proxy.example.com  ")).toBe("custom");
    });

    it("falls back to the provider id itself for an unknown provider", () => {
        expect(resolveEffectiveVendor("not-a-real-provider", undefined)).toBe("not-a-real-provider");
    });
});
