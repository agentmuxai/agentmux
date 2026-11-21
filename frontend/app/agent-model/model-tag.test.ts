// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { labelFromId, modelFamily, modelTag, readModelFacts, shortModelText } from "./model-tag";

describe("labelFromId", () => {
    it.each([
        ["claude-sonnet-5-5", "Sonnet 5.5"],
        ["claude-opus-5-5", "Opus 5.5"],
        ["claude-fable-5-2", "Fable 5.2"],
        ["claude-haiku-5-5", "Haiku 5.5"],
        ["claude-sonnet-4-5-20250929", "Sonnet 4.5"],
        ["claude-opus-4-1-latest", "Opus 4.1"],
        ["claude-opus-5-5[1m]", "Opus 5.5"],
        ["claude-opus-5", "Opus 5"],
        // not Claude's naming: as written
        ["gpt-6-astra", "gpt-6-astra"],
        ["gemini-3-pro", "gemini-3-pro"],
        ["openrouter/vendor/model", "openrouter/vendor/model"],
        ["llama3.1:8b", "llama3.1:8b"],
    ])("%s reads %s", (id, text) => {
        expect(labelFromId(id)).toBe(text);
    });
});

describe("shortModelText", () => {
    it("drops a leading Claude and cuts at 16 characters with an ellipsis", () => {
        expect(shortModelText("Claude Sonnet 5.5")).toBe("Sonnet 5.5");
        expect(shortModelText("claude-opus-5-5")).toBe("opus-5-5");
        expect(shortModelText("Gemini 3 Pro")).toBe("Gemini 3 Pro");
        const cut = shortModelText("openrouter/vendor/model");
        expect(cut).toBe("openrouter/vend…");
        expect(cut).toHaveLength(16);
        expect(shortModelText("Sixteen chars ok")).toBe("Sixteen chars ok");
    });
});

describe("modelFamily", () => {
    it.each([
        [{ label: "Fable 5.2" }, "fable"],
        [{ label: "Opus 5.5" }, "opus"],
        [{ id: "claude-sonnet-5-5" }, "sonnet"],
        [{ label: "HAIKU 5.5" }, "haiku"],
        [{ label: "GPT 6" }, "gpt"],
        [{ id: "gpt-6-astra" }, "gpt"],
        [{ id: "codex-mini-latest" }, "gpt"],
        [{ id: "o3-pro" }, "gpt"],
        [{ label: "5.1", provider: "codex" }, "gpt"],
        [{ label: "Gemini 3 Pro" }, "gemini"],
        [{ id: "gemini-3-pro" }, "gemini"],
        [{ label: "Pro", provider: "gemini" }, "gemini"],
        [{ id: "openrouter/vendor/model" }, "other"],
        [{ id: "llama3.1:8b" }, "other"],
        [{ label: "Claude Unknownium 9" }, "other"],
        // a route that carries a Claude tier in its id is that tier
        [{ id: "openrouter/anthropic/claude-opus-5-5" }, "opus"],
    ])("%j is %s", (facts, family) => {
        expect(modelFamily(facts)).toBe(family);
    });
});

describe("modelTag", () => {
    it("gives a solid tag in its family's hue for a model that is running", () => {
        const tag = modelTag({ label: "Sonnet 5.5", source: "effective", effort: "high" })!;
        expect(tag).toMatchObject({ text: "Sonnet 5.5", family: "sonnet", hue: "model-sonnet", muted: false, glyph: "star-of-life" });
        expect(tag.lines).toEqual(["Sonnet 5.5, running now (confirmed by its CLI). Effort high."]);
    });

    it("makes the label from the id when a build sends no label", () => {
        expect(modelTag({ id: "claude-opus-5-5", source: "observed" })).toMatchObject({
            text: "Opus 5.5",
            lines: ["Opus 5.5, from its last call."],
        });
    });

    it("gives each family group its own glyph, so colour is never the only cue", () => {
        const glyph = (label: string) => modelTag({ label })!.glyph;
        expect(glyph("Opus 5.5")).toBe("star-of-life");
        expect(glyph("GPT 6")).toBe("hexagon-nodes");
        expect(glyph("Gemini 3 Pro")).toBe("diamond");
        expect(glyph("llama3.1:8b")).toBe("circle");
    });

    it("mutes a model that was asked for and is not confirmed", () => {
        const tag = modelTag({ label: "Sonnet 5.5", source: "requested" })!;
        expect(tag).toMatchObject({ muted: true, glyph: "star-of-life", hue: "model-sonnet" });
        expect(tag.lines).toEqual(["Sonnet 5.5 was asked for; not confirmed yet."]);
    });

    it("mutes a pending switch, with an arrow, and says both models", () => {
        const tag = modelTag({ label: "Sonnet 5.5", source: "effective", pendingTo: "Opus 5.5" })!;
        expect(tag).toMatchObject({ muted: true, glyph: "arrow-right", text: "Sonnet 5.5" });
        expect(tag.lines).toEqual(["Switching to Opus 5.5 at its next turn. Now: Sonnet 5.5."]);
    });

    it("says what the definition says only when it differs", () => {
        expect(modelTag({ label: "Sonnet 5.5", definitionLabel: "Haiku 5.5" })!.lines).toContain("Its definition says Haiku 5.5.");
        expect(modelTag({ label: "Sonnet 5.5", definitionLabel: "sonnet 5.5" })!.lines).toHaveLength(1);
    });

    it("shows a model on a route as sure as a running one, and says so", () => {
        const tag = modelTag({ label: "GPT 6", source: "route", effort: "medium" })!;
        expect(tag).toMatchObject({ muted: false, family: "gpt", glyph: "hexagon-nodes" });
        expect(tag.lines).toEqual(["GPT 6, on its model route. Effort medium."]);
    });

    it("draws a label that srv calls unknown muted, like a request, and nothing without a label or an id", () => {
        expect(modelTag({ label: "Sonnet 5.5", source: "unknown" })).toMatchObject({ muted: true, text: "Sonnet 5.5" });
        expect(modelTag({ source: "unknown" })).toBeNull();
        expect(modelTag({})).toBeNull();
        expect(modelTag({ label: "  ", id: null })).toBeNull();
    });

    it("cuts a long label and keeps the whole one in the tooltip", () => {
        const tag = modelTag({ label: "Claude Sonnet 5.5 Extended Thinking" })!;
        expect(tag.text).toHaveLength(16);
        expect(tag.text.endsWith("…")).toBe(true);
        expect(tag.lines[0]).toContain("Claude Sonnet 5.5 Extended Thinking");
    });
});

describe("readModelFacts", () => {
    it("is null for a runtime row without the facts, so no tag is drawn", () => {
        expect(readModelFacts({ agent_id: "a1", state: "running", provider: "claude" })).toBeNull();
        expect(readModelFacts(null)).toBeNull();
        expect(readModelFacts("x")).toBeNull();
    });

    it("reads the label, source and a pending switch from a row that has them", () => {
        const facts = readModelFacts({
            agent_id: "a1",
            provider: "claude",
            model_label: "Opus 5.5",
            model_source: "effective",
            pending_switch: "Fable 5.2",
            definition_model_label: "Haiku 5.5",
        });
        expect(facts).toMatchObject({
            label: "Opus 5.5",
            source: "effective",
            pendingTo: "Fable 5.2",
            provider: "claude",
            definitionLabel: "Haiku 5.5",
        });
        expect(modelTag(facts!)).toMatchObject({ family: "opus", muted: true, glyph: "arrow-right" });
    });
});
