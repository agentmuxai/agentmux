// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { extractSearchResults, looksLikeSearchResults } from "./search-results";

describe("extractSearchResults", () => {
    it("reads a top-level array of result objects", () => {
        const out = extractSearchResults([
            { title: "Example", url: "https://example.com", snippet: "hi" },
            { title: "Two", url: "https://two.com" },
        ]);
        expect(out).toEqual([
            { title: "Example", url: "https://example.com", snippet: "hi", date: undefined, index: 1 },
            { title: "Two", url: "https://two.com", snippet: undefined, date: undefined, index: 2 },
        ]);
    });

    it("reads the array from results / content / items keys", () => {
        for (const key of ["results", "content", "items", "web_search_results"]) {
            const out = extractSearchResults({ [key]: [{ url: "https://a.com", title: "A" }] });
            expect(out).toEqual([{ title: "A", url: "https://a.com", snippet: undefined, date: undefined, index: 1 }]);
        }
    });

    it("tolerates field-name variants (link/uri, name/heading, description/text)", () => {
        expect(extractSearchResults([{ link: "https://l.com", name: "N", description: "D" }])).toEqual([
            { title: "N", url: "https://l.com", snippet: "D", date: undefined, index: 1 },
        ]);
        expect(extractSearchResults([{ uri: "https://u.com", heading: "H" }])).toEqual([
            { title: "H", url: "https://u.com", snippet: undefined, date: undefined, index: 1 },
        ]);
    });

    it("maps page_age to date field (not snippet)", () => {
        expect(extractSearchResults([{ url: "https://u.com", title: "T", page_age: "2 days" }])).toEqual([
            { title: "T", url: "https://u.com", snippet: undefined, date: "2 days", index: 1 },
        ]);
    });

    it("falls back title→url when no title field is present", () => {
        expect(extractSearchResults([{ url: "https://x.com" }])).toEqual([
            { title: "https://x.com", url: "https://x.com", snippet: undefined, date: undefined, index: 1 },
        ]);
    });

    it("skips items without a URL and returns null when none qualify", () => {
        expect(extractSearchResults([{ title: "no url" }, { foo: 1 }])).toBeNull();
        // mixed: only the URL-bearing item survives (index reflects position in original array)
        expect(extractSearchResults([{ title: "no url" }, { url: "https://y.com" }])).toEqual([
            { title: "https://y.com", url: "https://y.com", snippet: undefined, date: undefined, index: 2 },
        ]);
    });

    it("returns null for non-search results (structured, string, empty, nullish)", () => {
        expect(extractSearchResults({ status: "done", count: 3 })).toBeNull();
        expect(extractSearchResults("just a string")).toBeNull();
        expect(extractSearchResults({ content: "a string body" })).toBeNull(); // content is a string but not a JSON array
        expect(extractSearchResults([])).toBeNull();
        expect(extractSearchResults(null)).toBeNull();
        expect(extractSearchResults(undefined)).toBeNull();
    });

    it("parses JSON-encoded array string at top level", () => {
        const input = JSON.stringify([{ url: "https://a.com", title: "A" }]);
        expect(extractSearchResults(input)).toEqual([
            { title: "A", url: "https://a.com", snippet: undefined, date: undefined, index: 1 },
        ]);
    });

    it("parses JSON-encoded array string under a known key", () => {
        const input = { content: JSON.stringify([{ url: "https://b.com", title: "B", snippet: "S" }]) };
        expect(extractSearchResults(input)).toEqual([
            { title: "B", url: "https://b.com", snippet: "S", date: undefined, index: 1 },
        ]);
    });

    it("looksLikeSearchResults mirrors extract", () => {
        expect(looksLikeSearchResults([{ url: "https://a.com" }])).toBe(true);
        expect(looksLikeSearchResults({ a: 1 })).toBe(false);
    });
});
