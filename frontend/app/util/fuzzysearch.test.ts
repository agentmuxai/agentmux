// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Golden-set tests for the shared fuzzy-search utility
// (docs/specs/SPEC_SETTINGS_PANE_SEARCH_2026_09_21.md §6) — literal match,
// typo tolerance, and (the point of the Settings consumer) synonym matching
// via a `keywords` field. Runs the real Fuse.js call, not a mock, so a
// threshold/weight tuning change that breaks one of these is caught here.

import { describe, expect, it } from "vitest";

import { fuzzySearch } from "./fuzzysearch";

interface Entry {
    id: string;
    label: string;
    description?: string;
    keywords?: string[];
}

const ITEMS: Entry[] = [
    { id: "appearance.theme", label: "Theme", description: "Choose light, dark, or match your system.", keywords: ["dark mode", "light mode", "color scheme"] },
    { id: "terminal.ligatures", label: "Font Ligatures", description: "Render coding ligatures like => as a single glyph.", keywords: ["ligatures", "programming ligatures", "cursive code"] },
    { id: "advanced.idle_timeout", label: "Idle Timeout", description: "Lock the app after a period of inactivity.", keywords: ["auto-lock", "sleep", "inactivity timer"] },
    { id: "terminal.font_size", label: "Font Size", description: "Terminal text size.", keywords: ["text size", "zoom", "make text bigger"] },
];

const KEYS = [
    { name: "label", weight: 0.45 },
    { name: "keywords", weight: 0.35 },
    { name: "description", weight: 0.2 },
];

describe("fuzzySearch", () => {
    it("returns items unchanged, in original order, for a blank query", () => {
        expect(fuzzySearch(ITEMS, "", { keys: KEYS })).toBe(ITEMS);
        expect(fuzzySearch(ITEMS, "   ", { keys: KEYS })).toBe(ITEMS);
    });

    it("matches a literal label", () => {
        const results = fuzzySearch(ITEMS, "Theme", { keys: KEYS });
        expect(results[0]?.id).toBe("appearance.theme");
    });

    it("tolerates a typo in the label", () => {
        const results = fuzzySearch(ITEMS, "Ligatues", { keys: KEYS }); // missing 'r'
        expect(results[0]?.id).toBe("terminal.ligatures");
    });

    it("matches via a curated synonym not present in the label or description", () => {
        const results = fuzzySearch(ITEMS, "dark mode", { keys: KEYS });
        expect(results[0]?.id).toBe("appearance.theme");
    });

    it("matches a second synonym case", () => {
        const results = fuzzySearch(ITEMS, "auto-lock", { keys: KEYS });
        expect(results[0]?.id).toBe("advanced.idle_timeout");
    });

    it("matches a verb/goal phrasing synonym", () => {
        const results = fuzzySearch(ITEMS, "make text bigger", { keys: KEYS });
        expect(results[0]?.id).toBe("terminal.font_size");
    });

    it("returns no results for a query with nothing in common", () => {
        const results = fuzzySearch(ITEMS, "xyzzy_nonexistent_query", { keys: KEYS });
        expect(results).toHaveLength(0);
    });

    it("supports a function-based key (getFn), for domains with no fixed field to search", () => {
        interface AgentRow {
            instance_name?: string;
            definition_name: string;
        }
        const rows: AgentRow[] = [
            { instance_name: "Loap #2", definition_name: "claude" },
            { definition_name: "Opaz" },
        ];
        const results = fuzzySearch(rows, "Opaz", {
            keys: [{ name: "searchName", getFn: (r) => r.instance_name || r.definition_name }],
        });
        expect(results[0]?.definition_name).toBe("Opaz");
    });
});
