// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    extractSearchResults,
    extractWebSearch,
    groupLinksByDomain,
    looksLikeSearchResults,
    parseWebSearchText,
} from "./search-results";
import { CLAUDE_WEBSEARCH_RESULT } from "./websearch-fixture";

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

// SPEC_TOOL_PREVIEW_CONTENT_FIRST_2026_09_26.md §3.3 — Claude Code's WebSearch
// result is one prose string, never an array, so it needs its own parser.
describe("parseWebSearchText", () => {
    it("reads the real Claude Code result: query, links, summary; drops the REMINDER", () => {
        const parsed = parseWebSearchText(CLAUDE_WEBSEARCH_RESULT)!;
        expect(parsed.query).toBe("SolidJS createResource documentation");
        expect(parsed.links).toHaveLength(9);
        expect(parsed.links[0]).toEqual({
            title: "Fetching data - SolidJS Documentation",
            url: "https://docs.solidjs.com/guides/fetching-data",
            index: 1,
        });
        expect(parsed.summary!.startsWith("Here's what the SolidJS docs say")).toBe(true);
        expect(parsed.summary).toContain("## Official docs");
        expect(parsed.summary).not.toContain("REMINDER");
        expect(parsed.summary).not.toContain("Links:");
        expect(parsed.summary).not.toContain("Web search results for query");
    });

    it("merges several Links: lines and de-duplicates by URL", () => {
        const text = [
            'Web search results for query: "q"',
            'Links: [{"title":"A","url":"https://a.com"},{"title":"B","url":"https://b.com"}]',
            "",
            'Links: [{"title":"B again","url":"https://b.com"},{"title":"C","url":"https://c.com"}]',
            "",
            "Summary.",
        ].join("\n");
        const parsed = parseWebSearchText(text)!;
        expect(parsed.links.map((l) => l.url)).toEqual(["https://a.com", "https://b.com", "https://c.com"]);
        expect(parsed.links.map((l) => l.index)).toEqual([1, 2, 3]);
        expect(parsed.summary).toBe("Summary.");
    });

    it("skips a Links: line whose JSON does not parse", () => {
        const parsed = parseWebSearchText('Web search results for query: "q"\n\nLinks: [{oops\n\nText.')!;
        expect(parsed.links).toEqual([]);
        expect(parsed.summary).toBe("Text.");
    });

    it("returns null for a string that is not a search result", () => {
        expect(parseWebSearchText("hello world")).toBeNull();
        expect(parseWebSearchText('Web search results for query: "q"\n\nREMINDER: You MUST include the sources above.')).toBeNull();
    });

    it("extractWebSearch accepts the string and the translator's {content} wrapper", () => {
        expect(extractWebSearch(CLAUDE_WEBSEARCH_RESULT)?.links).toHaveLength(9);
        expect(extractWebSearch({ content: CLAUDE_WEBSEARCH_RESULT })?.links).toHaveLength(9);
        expect(extractWebSearch({ content: "plain" })).toBeNull();
        expect(extractWebSearch([{ url: "https://a.com", title: "A" }])).toBeNull();
    });
});

describe("groupLinksByDomain", () => {
    it("groups by hostname (www. stripped), first-seen order, with counts", () => {
        const groups = groupLinksByDomain(parseWebSearchText(CLAUDE_WEBSEARCH_RESULT)!.links);
        expect(groups.map((g) => [g.domain, g.links.length])).toEqual([
            ["docs.solidjs.com", 2],
            ["thisdot.co", 1],
            ["solidjs.com", 3],
            ["youtube.com", 1],
            ["vladislav-lipatov.medium.com", 1],
            ["raresportan.com", 1],
        ]);
    });
});
