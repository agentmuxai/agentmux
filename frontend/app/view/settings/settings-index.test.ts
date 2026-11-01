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

import { SETTINGS_INDEX, searchSettings, settingsPath } from "./settings-index";

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

    it("covers every section", () => {
        const sections = new Set(SETTINGS_INDEX.map((e) => e.section));
        expect(sections).toEqual(
            new Set(["appearance", "animations", "window", "browser", "terminal", "sounds", "notifications", "recording", "devices", "widgets", "agents", "advanced"]),
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

    it("finds the Supervisor's one entry by its settings' words, and it opens the Swarm", () => {
        for (const q of ["supervisor", "steady", "wait before continuing", "fleet budget", "cooldown", "supervisor:hourly"]) {
            expect(labels(q)).toContain("Swarm Supervisor");
        }
        const entry = SETTINGS_INDEX.find((e) => e.id === "agents.supervisor")!;
        expect(typeof entry.opens).toBe("function");
        expect(SETTINGS_INDEX.filter((e) => e.label.toLowerCase().includes("supervisor"))).toHaveLength(1);
    });

    it("finds the Kanban reminder switch by kanban, backlog and reminder", () => {
        for (const q of ["kanban", "backlog", "reminder"]) {
            expect(labels(q)).toContain("Remind idle agents of their backlog");
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

describe("the path of a setting", () => {
    const path = (e: { section: never; group?: string; label: string }) => settingsPath(e).join(" › ");
    const volumes = () => searchSettings(SETTINGS_INDEX, "volume").filter((e) => e.label === "Volume");

    it("is its section, the heading it sits under and its own name", () => {
        expect(settingsPath({ section: "sounds", label: "Volume" })).toEqual(["Sounds", "Volume"]);
        expect(settingsPath({ section: "sounds", group: "Tool-call tones", label: "Volume" })).toEqual([
            "Sounds",
            "Tool-call tones",
            "Volume",
        ]);
        // A heading that repeats the section or the setting's own name adds nothing.
        expect(settingsPath({ section: "sounds", group: "Sounds", label: "Volume" })).toEqual(["Sounds", "Volume"]);
        expect(settingsPath({ section: "browser", group: "Profiles", label: "Profiles" })).toEqual(["Browser", "Profiles"]);
    });

    it("tells the three Volume settings apart", () => {
        const paths = volumes().map((e) => path(e as never));
        expect(paths).toHaveLength(3);
        expect(new Set(paths).size).toBe(3);
        expect(paths).toEqual(
            expect.arrayContaining(["Sounds › Volume", "Sounds › Tool-call tones › Volume", "Sounds › Waiting for input › Volume"])
        );
    });

    it("is unique for every setting in the index, so no two results read the same", () => {
        const seen = new Map<string, string>();
        for (const e of SETTINGS_INDEX) {
            const p = settingsPath(e).join(" › ");
            expect(seen.get(p), `"${p}" is the path of both ${seen.get(p)} and ${e.id}`).toBeUndefined();
            seen.set(p, e.id);
        }
    });

    it("is searched too: the section and the heading find a setting, in one word or several", () => {
        const only = (q: string) => searchSettings(SETTINGS_INDEX, q).map((e) => path(e as never));
        expect(only("sounds volume")).toEqual(
            expect.arrayContaining(["Sounds › Volume", "Sounds › Tool-call tones › Volume", "Sounds › Waiting for input › Volume"])
        );
        const tool = only("tool-call tones volume");
        expect(tool[0]).toBe("Sounds › Tool-call tones › Volume");
        const waiting = only("waiting volume");
        expect(waiting).toContain("Sounds › Waiting for input › Volume");
        expect(waiting).not.toContain("Sounds › Tool-call tones › Volume");
    });

    it("leaves the order of single-word searches as it was", () => {
        expect(labels("opacity")).toEqual(["Opacity", "Magnified opacity", "Terminal transparency"]);
    });
});
