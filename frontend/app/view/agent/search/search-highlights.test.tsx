// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The words a search finds are painted with CSS.highlights (SPEC_AGENT_PANE_SEARCH_AND_DIM_MODE_2026_11_06 §4.4): every
 * occurrence in a mounted matching row in `agent-search`, those in the current row in `agent-search-current`, taken
 * back when the row unmounts or the query changes, and nothing painted where the API is missing.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { DocumentRow } from "../virtualization/DocumentRow";
import { findRanges, HIGHLIGHT_ALL, HIGHLIGHT_CURRENT, MAX_RANGES_PER_ROW, paintRanges } from "./search-highlights";
import { SearchPaintContext, type SearchPaint } from "./search-paint";
import type { DocumentNode, DocumentState } from "../../../agent-model/types";

class FakeHighlight extends Set<Range> {}

let registry: Map<string, FakeHighlight>;

beforeEach(() => {
    registry = new Map();
    vi.stubGlobal("CSS", { highlights: registry });
    vi.stubGlobal("Highlight", FakeHighlight);
});

afterEach(() => {
    cleanup();
    vi.unstubAllGlobals();
});

const textsOf = (name: string): string[] => [...(registry.get(name) ?? [])].map((r) => r.toString());

function el(html: string): HTMLElement {
    const root = document.createElement("div");
    root.innerHTML = html;
    document.body.append(root);
    return root;
}

describe("finding the words", () => {
    it("finds every occurrence, folding case as the index does, across text nodes", () => {
        const root = el("<p>Find the FIND and <em>find</em> again</p>");
        expect(findRanges(root, "find").map((r) => r.toString())).toEqual(["Find", "FIND", "find"]);
    });

    it("finds nothing for an empty query and skips a text whose fold changes its length", () => {
        const root = el("<p>İ find</p><p>find</p>");
        expect(findRanges(root, "")).toEqual([]);
        // The first paragraph folds to a longer string; only the second is painted.
        expect(findRanges(root, "find")).toHaveLength(1);
    });

    it("stops at the cap on a very long row", () => {
        const root = el(`<p>${"a ".repeat(MAX_RANGES_PER_ROW + 50)}</p>`);
        expect(findRanges(root, "a")).toHaveLength(MAX_RANGES_PER_ROW);
    });

    it("does not paint a match that spans two nodes", () => {
        const root = el("<p>fi<em>nd</em></p>");
        expect(findRanges(root, "find")).toEqual([]);
    });
});

describe("painting", () => {
    it("puts ranges in the all or the current highlight and takes them back", () => {
        const root = el("<p>one one</p>");
        const ranges = findRanges(root, "one");
        const releaseAll = paintRanges(ranges, false);
        expect(registry.get(HIGHLIGHT_ALL)?.size).toBe(2);
        releaseAll();
        expect(registry.get(HIGHLIGHT_ALL)?.size).toBe(0);
        const releaseCurrent = paintRanges(ranges, true);
        expect(registry.get(HIGHLIGHT_CURRENT)?.size).toBe(2);
        releaseCurrent();
        expect(registry.get(HIGHLIGHT_CURRENT)?.size).toBe(0);
    });

    it("paints nothing where the API is missing", () => {
        vi.stubGlobal("CSS", {});
        const root = el("<p>one</p>");
        expect(() => paintRanges(findRanges(root, "one"), false)()).not.toThrow();
        expect(registry.size).toBe(0);
    });
});

const docState = (): DocumentState =>
    ({ collapsedNodes: new Set(), pinnedNodes: new Set(), heldOpenNodes: new Set() }) as unknown as DocumentState;
const md = (id: string, content: string): DocumentNode => ({ type: "markdown", id, content, timestamp: 0 });

function mountRows(nodes: DocumentNode[]) {
    const [matches, setMatches] = createSignal<ReadonlySet<string>>(new Set());
    const [searched, setSearched] = createSignal("");
    const [currentId, setCurrentId] = createSignal<string | null>(null);
    const paint: SearchPaint = { matches, searched, dim: () => false, currentId };
    const [state] = createSignal(docState());
    const utils = render(() => (
        <SearchPaintContext.Provider value={paint}>
            {nodes.map((n) => (
                <DocumentRow node={() => n} documentState={state} onToggleCollapse={() => {}} onTogglePin={() => {}} />
            ))}
        </SearchPaintContext.Provider>
    ));
    return { utils, setMatches, setSearched, setCurrentId };
}

describe("a mounted row", () => {
    it("paints the words of the rows that match, and the current row's in the current highlight", () => {
        const s = mountRows([md("a", "alpha beta"), md("b", "beta gamma beta"), md("c", "delta")]);
        s.setSearched("beta");
        s.setMatches(new Set(["a", "b"]));
        s.setCurrentId("b");
        expect(textsOf(HIGHLIGHT_ALL)).toEqual(["beta"]);
        expect(textsOf(HIGHLIGHT_CURRENT)).toEqual(["beta", "beta"]);
    });

    it("paints nothing in a row that is not a match, even if its text has the query", () => {
        const s = mountRows([md("a", "alpha beta")]);
        s.setSearched("beta");
        expect(registry.get(HIGHLIGHT_ALL)?.size ?? 0).toBe(0);
        s.setMatches(new Set(["a"]));
        expect(registry.get(HIGHLIGHT_ALL)?.size).toBe(1);
    });

    it("follows the query and clears when it ends", () => {
        const s = mountRows([md("a", "alpha beta")]);
        s.setMatches(new Set(["a"]));
        s.setSearched("alpha");
        expect(textsOf(HIGHLIGHT_ALL)).toEqual(["alpha"]);
        s.setSearched("beta");
        expect(textsOf(HIGHLIGHT_ALL)).toEqual(["beta"]);
        s.setSearched("");
        expect(registry.get(HIGHLIGHT_ALL)?.size).toBe(0);
    });

    it("takes its ranges back when it unmounts", () => {
        const s = mountRows([md("a", "alpha beta")]);
        s.setMatches(new Set(["a"]));
        s.setSearched("alpha");
        expect(registry.get(HIGHLIGHT_ALL)?.size).toBe(1);
        s.utils.unmount();
        expect(registry.get(HIGHLIGHT_ALL)?.size).toBe(0);
    });

    it("carries data-search-match by id and marks only the current row aria-current", () => {
        const s = mountRows([md("a", "alpha"), md("b", "beta")]);
        s.setMatches(new Set(["b"]));
        const rows = () => [...s.utils.container.querySelectorAll<HTMLElement>(".agent-document-row")];
        expect(rows().map((r) => r.dataset.searchMatch)).toEqual(["false", "true"]);
        expect(rows().some((r) => r.hasAttribute("aria-current"))).toBe(false);
    });
});
