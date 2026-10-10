// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests against the real, production SETTINGS_INDEX (not a synthetic
// fixture) — docs/specs/SPEC_SETTINGS_PANE_SEARCH_2026_09_21.md §6's
// "golden set," run against the actual authored keyword lists so a
// threshold/weight tuning change (or a typo in a keyword list) that breaks
// a real synonym case is caught here, not just in fuzzysearch.test.ts's
// synthetic fixture. Runs `searchSettings`, the search the Settings pane
// itself uses.

import { describe, expect, it } from "vitest";

import { SETTINGS_INDEX, searchSettings } from "./settings-index";

const labels = (query: string) => searchSettings(SETTINGS_INDEX, query).map((e) => e.label);

describe("SETTINGS_INDEX", () => {
    it("has no duplicate ids", () => {
        const ids = SETTINGS_INDEX.map((e) => e.id);
        expect(new Set(ids).size).toBe(ids.length);
    });

    it("gives every entry a non-empty label and at least one keyword", () => {
        for (const entry of SETTINGS_INDEX) {
            expect(entry.label.length).toBeGreaterThan(0);
            expect(entry.keywords.length).toBeGreaterThan(0);
        }
    });

    it("covers all nine sections", () => {
        const sections = new Set(SETTINGS_INDEX.map((e) => e.section));
        expect(sections).toEqual(
            new Set(["appearance", "window", "browser", "terminal", "sounds", "notifications", "recording", "devices", "widgets", "advanced"]),
        );
    });

    // Golden set: real synonym queries against the real authored data.
    it.each([
        ["dark mode", "appearance.theme"],
        ["auto-lock", "terminal.agent_idle_timeout"],
        ["make text bigger", "terminal.font_size"],
        ["boot screen", "appearance.splash"],
        ["gpu rendering", "advanced.disable_webgl"],
        ["clipboard", "terminal.copy_on_select"],
    ])('finds the right setting for %j', (query, expectedId) => {
        const results = searchSettings(SETTINGS_INDEX, query);
        expect(results[0]?.id).toBe(expectedId);
    });
});

// Literal first: what the user typed counts before how close it looks.
describe("searchSettings", () => {
    it("returns the entries unchanged for a blank query", () => {
        expect(searchSettings(SETTINGS_INDEX, "  ")).toBe(SETTINGS_INDEX);
    });

    it("finds the MuxBus Cloud status-bar toggle by its synonyms", () => {
        for (const q of ["muxbus", "cloud dot", "hide cloud"]) {
            expect(labels(q)).toContain("Show MuxBus Cloud in the status bar");
        }
    });

    it("lists an exact setting name first", () => {
        expect(labels("Theme")[0]).toBe("Theme");
        expect(labels("model")[0]).toBe("Model");
    });

    it("does not list near-misses that do not contain the query", () => {
        // A plain fuzzy search also listed "Message rejected" and "Agent
        // stopped with an error" for these.
        expect(labels("Message accepted")).toEqual(["Message accepted"]);
        expect(labels("Turn error")).toEqual(["Turn error"]);
        expect(labels("Font size")).toEqual(["Font size"]);
    });

    it("ranks label matches above keyword and description matches", () => {
        // Terminal transparency only has "opacity" as a keyword.
        expect(labels("opacity")).toEqual(["Opacity", "Magnified opacity", "Terminal transparency"]);
    });

    it("still finds a setting through a typo", () => {
        expect(labels("Copy on selct")[0]).toBe("Copy on select");
        expect(labels("Opactiy")[0]).toBe("Opacity");
    });
});
