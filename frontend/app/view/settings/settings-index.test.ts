// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests against the real, production SETTINGS_INDEX (not a synthetic
// fixture) — docs/specs/SPEC_SETTINGS_PANE_SEARCH_2026_09_21.md §6's
// "golden set," run against the actual authored keyword lists so a
// threshold/weight tuning change (or a typo in a keyword list) that breaks
// a real synonym case is caught here, not just in fuzzysearch.test.ts's
// synthetic fixture.

import { describe, expect, it } from "vitest";

import { fuzzySearch } from "@/app/util/fuzzysearch";
import { SETTINGS_INDEX } from "./settings-index";

const SEARCH_KEYS = [
    { name: "label", weight: 0.45 },
    { name: "keywords", weight: 0.35 },
    { name: "description", weight: 0.2 },
];

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

    it("covers all seven sections", () => {
        const sections = new Set(SETTINGS_INDEX.map((e) => e.section));
        expect(sections).toEqual(
            new Set(["appearance", "window", "terminal", "sounds", "notifications", "recording", "advanced"]),
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
        const results = fuzzySearch(SETTINGS_INDEX, query, { keys: SEARCH_KEYS });
        expect(results[0]?.id).toBe(expectedId);
    });
});
