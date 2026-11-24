// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// What a door card's caption says about billing, by how the door signs in
// (docs/specs/SPEC_AGENT_HARNESS_CURATION_FIRST_CLASS_AND_PREVIEW_2026_11_07.md §7, F2).
import { describe, expect, it } from "vitest";
import { getCliCatalogEntry } from "./cli-catalog";

const blurb = (id: string) => getCliCatalogEntry(id)?.blurb ?? "";

describe("door card captions: billing by sign-in class", () => {
    it("the lab-account doors whose plan is the expected bill say so", () => {
        expect(blurb("claude")).toMatch(/on your Claude plan$/);
        expect(blurb("codex")).toMatch(/on your ChatGPT plan$/);
        expect(blurb("copilot")).toMatch(/on your Copilot plan$/);
    });

    it("grok and muse say how they sign in and make no claim about a plan", () => {
        expect(blurb("grok")).toMatch(/signs in with your xAI account$/);
        expect(blurb("muse")).toMatch(/signs in with your Meta account$/);
        for (const id of ["grok", "muse"]) expect(blurb(id).toLowerCase()).not.toContain("plan");
    });

    it("the key doors say the key is the bill and never mention a plan", () => {
        expect(blurb("kimi")).toMatch(/with your Moonshot key$/);
        expect(blurb("qwen")).toMatch(/with your own key$/);
        expect(blurb("gemini")).toMatch(/with a Gemini API key$/);
        for (const id of ["kimi", "qwen", "gemini"]) expect(blurb(id).toLowerCase()).not.toContain("plan");
    });

    it("only the three plan doors use the word plan", () => {
        const withPlan = ["claude", "codex", "gemini", "antigravity", "qwen", "kimi", "pi", "openclaw", "copilot", "kilo", "cline", "opencode", "goose", "grok", "dsh", "aider", "crush", "muse"]
            .filter((id) => blurb(id).toLowerCase().includes("plan"))
            .sort();
        // Cline's "Plan-then-act" is its mode, not a billing plan.
        expect(withPlan).toEqual(["claude", "cline", "codex", "copilot"]);
    });
});
