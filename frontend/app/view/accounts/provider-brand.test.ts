// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { brandForProvider, isCliOAuthProvider } from "./provider-brand";

describe("brandForProvider", () => {
    it("maps CLI-OAuth provider ids onto their brand", () => {
        expect(brandForProvider("claude")).toBe("anthropic");
        expect(brandForProvider("codex")).toBe("openai");
        expect(brandForProvider("gemini")).toBe("google");
        expect(brandForProvider("copilot")).toBe("github");
    });

    it("passes brand ids through unchanged", () => {
        for (const brand of ["anthropic", "openai", "google", "github", "aws", "slack", "custom", "agentmux"]) {
            expect(brandForProvider(brand)).toBe(brand);
        }
    });

    it("passes unmapped providers through unchanged", () => {
        expect(brandForProvider("openclaw")).toBe("openclaw");
        expect(brandForProvider("kimi")).toBe("kimi");
    });

    it("flags CLI-OAuth providers, not brands", () => {
        expect(isCliOAuthProvider("claude")).toBe(true);
        expect(isCliOAuthProvider("copilot")).toBe(true);
        expect(isCliOAuthProvider("anthropic")).toBe(false);
        expect(isCliOAuthProvider("github")).toBe(false);
    });
});
