// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { cleanProviderLabel, defaultAgentName } from "./default-agent-name";

describe("cleanProviderLabel", () => {
    it("strips a trailing ' Code' suffix", () => {
        expect(cleanProviderLabel("Claude Code")).toBe("Claude");
    });

    it("strips a trailing ' CLI' suffix", () => {
        expect(cleanProviderLabel("Codex CLI")).toBe("Codex");
    });

    it("strips a trailing ' Code CLI' suffix", () => {
        expect(cleanProviderLabel("Kimi Code CLI")).toBe("Kimi");
    });

    it("leaves a multi-word display name with no suffix unchanged", () => {
        expect(cleanProviderLabel("GitHub Copilot CLI")).toBe("GitHub Copilot");
    });

    it("leaves a display name with no matching suffix unchanged", () => {
        expect(cleanProviderLabel("OpenClaw")).toBe("OpenClaw");
        expect(cleanProviderLabel("Pi")).toBe("Pi");
    });
});

describe("defaultAgentName", () => {
    it("returns '<Provider> Agent' when unused", () => {
        expect(defaultAgentName("Claude Code", new Set())).toBe("Claude Agent");
    });

    it("suffixes 2 when the base name is taken", () => {
        expect(defaultAgentName("Claude Code", new Set(["Claude Agent"]))).toBe("Claude Agent 2");
    });

    it("takes the lowest unused suffix against the live set — a gap (here, a never-used '3') is used before climbing past the taken '4'", () => {
        const existing = new Set(["Claude Agent", "Claude Agent 2", "Claude Agent 4"]);
        expect(defaultAgentName("Claude Code", existing)).toBe("Claude Agent 3");
    });

    it("reuses a freed name — existingNames is a live snapshot, not persisted counter state, so a since-deleted 'Claude Agent 2' is returned again", () => {
        const existing = new Set(["Claude Agent"]); // "Claude Agent 2" once existed, since deleted
        expect(defaultAgentName("Claude Code", existing)).toBe("Claude Agent 2");
    });

    it("keeps climbing past a long run of taken names", () => {
        const existing = new Set(["Claude Agent", "Claude Agent 2", "Claude Agent 3", "Claude Agent 4"]);
        expect(defaultAgentName("Claude Code", existing)).toBe("Claude Agent 5");
    });

    it("is unaffected by names for a different provider", () => {
        const existing = new Set(["Codex Agent"]);
        expect(defaultAgentName("Claude Code", existing)).toBe("Claude Agent");
    });
});
