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
            { title: "Example", url: "https://example.com", snippet: "hi" },
            { title: "Two", url: "https://two.com", snippet: undefined },
        ]);
    });

    it("reads the array from results / content / items keys", () => {
        for (const key of ["results", "content", "items", "web_search_results"]) {
            const out = extractSearchResults({ [key]: [{ url: "https://a.com", title: "A" }] });
            expect(out).toEqual([{ title: "A", url: "https://a.com", snippet: undefined }]);
        }
    });

    it("tolerates field-name variants (link/uri, name/heading, description/text/page_age)", () => {
        expect(extractSearchResults([{ link: "https://l.com", name: "N", description: "D" }])).toEqual([
            { title: "N", url: "https://l.com", snippet: "D" },
        ]);
        expect(extractSearchResults([{ uri: "https://u.com", heading: "H", page_age: "2 days" }])).toEqual([
            { title: "H", url: "https://u.com", snippet: "2 days" },
        ]);
    });

    it("falls back title→url when no title field is present", () => {
        expect(extractSearchResults([{ url: "https://x.com" }])).toEqual([
            { title: "https://x.com", url: "https://x.com", snippet: undefined },
        ]);
    });

    it("skips items without a URL and returns null when none qualify", () => {
        expect(extractSearchResults([{ title: "no url" }, { foo: 1 }])).toBeNull();
        // mixed: only the URL-bearing item survives
        expect(extractSearchResults([{ title: "no url" }, { url: "https://y.com" }])).toEqual([
            { title: "https://y.com", url: "https://y.com", snippet: undefined },
        ]);
    });

    it("returns null for non-search results (structured, string, empty, nullish)", () => {
        expect(extractSearchResults({ status: "done", count: 3 })).toBeNull();
        expect(extractSearchResults("just a string")).toBeNull();
        expect(extractSearchResults({ content: "a string body" })).toBeNull(); // content is a string, not an array
        expect(extractSearchResults([])).toBeNull();
        expect(extractSearchResults(null)).toBeNull();
        expect(extractSearchResults(undefined)).toBeNull();
    });

    it("looksLikeSearchResults mirrors extract", () => {
        expect(looksLikeSearchResults([{ url: "https://a.com" }])).toBe(true);
        expect(looksLikeSearchResults({ a: 1 })).toBe(false);
    });
});
