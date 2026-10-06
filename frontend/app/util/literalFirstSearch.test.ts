// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `literalFirstSearch`: a query that appears in a name matches by what it says,
// not by how close it is. Typing "agentx" must not list "agenty" while
// "agentx" or "agentxx" exist. Runs the real Fuse.js fallback, not a mock.

import { describe, expect, it } from "vitest";

import { literalFirstSearch } from "./fuzzysearch";

const names = (items: { name: string }[]) => items.map((i) => i.name);
const search = (items: { name: string }[], q: string) =>
    literalFirstSearch(items, q, (i) => i.name, { keys: [{ name: "name", getFn: (i) => i.name }] });
const of = (...n: string[]) => n.map((name) => ({ name }));

describe("literalFirstSearch", () => {
    it("returns items unchanged, in original order, for a blank query", () => {
        const items = of("b", "a");
        expect(search(items, "")).toBe(items);
        expect(search(items, "   ")).toBe(items);
    });

    it("does not list a near-miss when the exact name exists (agentx vs agenty)", () => {
        expect(names(search(of("agentx", "agenty"), "agentx"))).toEqual(["agentx"]);
    });

    it("lists every name that contains the query, and nothing that only looks like it", () => {
        expect(names(search(of("agenty", "agentxx", "agentx", "Other"), "agentx"))).toEqual(["agentx", "agentxx"]);
    });

    it("is case-insensitive", () => {
        expect(names(search(of("AgentX", "AGENTY"), "agentx"))).toEqual(["AgentX"]);
        expect(names(search(of("agentx"), "AGENTX"))).toEqual(["agentx"]);
    });

    it("trims the query", () => {
        expect(names(search(of("agentx", "agenty"), "  agentx  "))).toEqual(["agentx"]);
    });

    it("matches anywhere in the name", () => {
        expect(names(search(of("My Agentx Two", "Maks"), "agentx"))).toEqual(["My Agentx Two"]);
    });

    it("ranks an exact name first, then a prefix, then a word start, then the rest", () => {
        const items = of("zz agentx", "contains-xagentxy", "agentxx", "agentx", "a agentx b");
        expect(names(search(items, "agentx"))).toEqual([
            "agentx",
            "agentxx",
            "zz agentx",
            "a agentx b",
            "contains-xagentxy",
        ]);
    });

    it("keeps the original order among equally ranked names", () => {
        expect(names(search(of("agentxb", "agentxa", "agentxc"), "agentx"))).toEqual(["agentxb", "agentxa", "agentxc"]);
    });

    it("falls back to typo tolerance only when nothing contains the query", () => {
        expect(names(search(of("agenty", "Other"), "agentz"))).toEqual(["agenty"]);
    });

    it("returns nothing when neither a literal nor a fuzzy match exists", () => {
        expect(search(of("Maks"), "zzz-no-such-agent")).toEqual([]);
    });
});

// The command palette and Settings search give each item its name plus other
// words it is known by (keywords, category, id, description).
describe("literalFirstSearch with other words per item", () => {
    interface Entry {
        name: string;
        words?: string[];
    }
    const searchAll = (items: Entry[], q: string) =>
        literalFirstSearch(items, q, (i) => [i.name, ...(i.words ?? [])], {
            keys: [
                { name: "name", weight: 0.6 },
                { name: "words", weight: 0.4 },
            ],
        });

    it("matches an item by one of its other words", () => {
        const items: Entry[] = [{ name: "Theme", words: ["dark mode"] }, { name: "Font size", words: ["zoom"] }];
        expect(names(searchAll(items, "dark mode"))).toEqual(["Theme"]);
    });

    it("ranks every name match above any match in other words, even an exact one", () => {
        const items: Entry[] = [
            { name: "Terminal transparency", words: ["opacity"] },
            { name: "Magnified opacity" },
            { name: "Opacity", words: ["window opacity"] },
        ];
        expect(names(searchAll(items, "opacity"))).toEqual(["Opacity", "Magnified opacity", "Terminal transparency"]);
    });

    it("orders other-word matches by each item's best word", () => {
        const items: Entry[] = [
            { name: "A", words: ["has opacity inside"] },
            { name: "B", words: ["opacityish", "opacity"] },
            { name: "C", words: ["opacity level"] },
        ];
        expect(names(searchAll(items, "opacity"))).toEqual(["B", "C", "A"]);
    });

    it("skips blank other words", () => {
        const items = [{ name: "agentx" }, { name: "Other" }];
        const result = literalFirstSearch(items, "agentx", (i) => [i.name, undefined, null, ""], {
            keys: [{ name: "name", getFn: (i) => i.name }],
        });
        expect(names(result)).toEqual(["agentx"]);
    });

    it("does not list a near-miss in other words while something contains the query", () => {
        const items: Entry[] = [{ name: "Other", words: ["agenty"] }, { name: "agentx" }];
        expect(names(searchAll(items, "agentx"))).toEqual(["agentx"]);
    });

    it("falls back to typo tolerance only when no name or other word contains the query", () => {
        const items: Entry[] = [{ name: "Theme", words: ["dark mode"] }, { name: "Font size", words: ["zoom"] }];
        expect(names(searchAll(items, "drak mode"))[0]).toBe("Theme");
    });
});
