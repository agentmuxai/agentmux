// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Dim mode paints and does nothing else (SPEC_AGENT_PANE_SEARCH_AND_DIM_MODE_2026_11_06 §4.4, §6): the scroller says
 * whether dim applies, each row says whether it matches by its node id, and turning dim on or off or changing the
 * matches moves no row, mounts or unmounts none, and starts no measurement. jsdom has no layout, so "moves no row"
 * is the rows' inline position and the observers' work staying as they were.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { AgentDocumentVirtualList } from "./AgentDocumentVirtualList";
import { createAgentViewState } from "./state";
import { SearchPaintContext, type SearchPaint } from "../search/search-paint";
import type { DocumentNode, DocumentState } from "../../../agent-model/types";

afterEach(() => cleanup());

const observed: Element[] = [];
class CountingResizeObserver {
    observe(el: Element): void {
        observed.push(el);
    }
    unobserve(): void {}
    disconnect(): void {}
}

const documentState = (): DocumentState => ({
    collapsedNodes: new Set(),
    pinnedNodes: new Set(),
    heldOpenNodes: new Set(),
    scrollPosition: 0,
    selectedNode: null,
    filter: { showThinking: true } as DocumentState["filter"],
});

const md = (id: string): DocumentNode => ({ type: "markdown", id, content: `text ${id}`, timestamp: 0 });

function setup() {
    const [nodes] = createSignal<DocumentNode[]>([md("a"), md("b"), md("c")]);
    const [matches, setMatches] = createSignal<ReadonlySet<string>>(new Set());
    const [dim, setDim] = createSignal(false);
    const [searched, setSearched] = createSignal("");
    const paint: SearchPaint = { matches, searched, dim, currentId: () => null };
    const viewState = createAgentViewState(nodes);
    const [docState] = createSignal(documentState());
    const utils = render(() => (
        <SearchPaintContext.Provider value={paint}>
            <AgentDocumentVirtualList viewState={viewState} documentState={docState} onToggleCollapse={() => {}} onTogglePin={() => {}} />
        </SearchPaintContext.Provider>
    ));
    const scroller = () => utils.container.querySelector(".agent-document") as HTMLElement;
    const rows = () => [...utils.container.querySelectorAll<HTMLElement>(".agent-document-row")];
    return { scroller, rows, setMatches, setDim, setSearched };
}

describe("dim mode in the transcript", () => {
    beforeEach(() => {
        observed.length = 0;
        vi.stubGlobal("ResizeObserver", CountingResizeObserver);
        vi.spyOn(console, "info").mockImplementation(() => {});
    });

    it("the scroller carries data-search-dim only while dim applies", () => {
        const s = setup();
        expect(s.scroller().hasAttribute("data-search-dim")).toBe(false);
        s.setDim(true);
        expect(s.scroller().hasAttribute("data-search-dim")).toBe(true);
        s.setDim(false);
        expect(s.scroller().hasAttribute("data-search-dim")).toBe(false);
    });

    it("each row says whether it matches, by node id", () => {
        const s = setup();
        expect(s.rows().length).toBeGreaterThan(0);
        s.setMatches(new Set(["b"]));
        const flags = Object.fromEntries(s.rows().map((r) => [r.dataset.nodeId, r.dataset.searchMatch]));
        expect(flags).toEqual({ a: "false", b: "true", c: "false" });
        s.setMatches(new Set(["a", "c"]));
        expect(Object.fromEntries(s.rows().map((r) => [r.dataset.nodeId, r.dataset.searchMatch]))).toEqual({
            a: "true",
            b: "false",
            c: "true",
        });
    });

    it("toggling dim and changing the matches moves no row, mounts or unmounts none, and measures nothing new", () => {
        const s = setup();
        const before = s.rows();
        const styles = before.map((r) => r.getAttribute("style"));
        const watched = observed.length;
        s.setSearched("text");
        s.setMatches(new Set(["b"]));
        s.setDim(true);
        s.setMatches(new Set(["a"]));
        s.setDim(false);
        const after = s.rows();
        expect(after).toEqual(before);
        expect(after.map((r) => r.getAttribute("style"))).toEqual(styles);
        expect(observed.length).toBe(watched);
    });
});
