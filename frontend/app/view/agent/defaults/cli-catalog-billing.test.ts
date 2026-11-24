// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// What a door card's caption says about billing, by how the door signs in
// (docs/specs/SPEC_AGENT_HARNESS_CURATION_FIRST_CLASS_AND_PREVIEW_2026_11_07.md §7, F2).
import { describe, expect, it } from "vitest";
import { cardCaption, getCliCatalogEntry } from "./cli-catalog";

const billing = (id: string) => getCliCatalogEntry(id)?.billing;
const caption = (id: string) => cardCaption(getCliCatalogEntry(id));

describe("door card captions: billing by sign-in class", () => {
    it("the lab-account doors whose plan is the expected bill name the plan", () => {
        expect(billing("claude")).toBe("Your Claude plan");
        expect(billing("codex")).toBe("Your ChatGPT plan");
        expect(billing("copilot")).toBe("Your Copilot plan");
    });

    it("grok and muse say how they sign in and make no claim about a plan", () => {
        expect(billing("grok")).toBe("xAI sign-in");
        expect(billing("muse")).toBe("Meta sign-in");
    });

    it("the key doors say the key is the bill", () => {
        expect(billing("kimi")).toBe("Your Moonshot key");
        expect(billing("qwen")).toBe("Your own key");
    });

    it("only the three plan doors use the word plan, and no harness without a claim has a billing line", () => {
        const ids = ["claude", "codex", "gemini", "antigravity", "qwen", "kimi", "pi", "openclaw", "copilot", "kilo", "cline", "opencode", "goose", "grok", "dsh", "aider", "crush", "muse", "muxcode"];
        expect(ids.filter((id) => billing(id)?.toLowerCase().includes("plan")).sort()).toEqual(["claude", "codex", "copilot"]);
        // The preview harnesses and Gemini (its blurb already says it takes a key) carry none.
        expect(ids.filter((id) => !billing(id)).sort()).toEqual(["aider", "antigravity", "cline", "crush", "dsh", "gemini", "goose", "kilo", "muxcode", "openclaw", "opencode", "pi"]);
    });

    it("the caption leads with the billing words and the blurb is left as it was", () => {
        expect(caption("claude")).toBe("Your Claude plan · Anthropic's coding agent");
        expect(getCliCatalogEntry("claude")?.blurb).toBe("Anthropic's coding agent");
        expect(caption("gemini")).toBe(getCliCatalogEntry("gemini")?.blurb);
        expect(cardCaption(null)).toBe("");
    });
});
