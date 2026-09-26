// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const openExternal = vi.fn();
vi.mock("@/store/global", () => ({ getApi: () => ({ openExternal }) }));

import { SearchResults } from "./SearchResults";
import { resolveToolRenderer } from "./registry";
import { CLAUDE_WEBSEARCH_RESULT } from "./websearch-fixture";
import type { ToolNode } from "../../types";

afterEach(() => {
    cleanup();
    openExternal.mockClear();
});

const node = (result: unknown, over: Partial<ToolNode> = {}): ToolNode => ({
    type: "tool",
    id: "t1",
    tool: "Other",
    toolName: "WebSearch",
    params: {},
    status: "success",
    collapsed: true,
    summary: "x",
    result: result as any,
    ...over,
});

describe("SearchResults", () => {
    it("renders a card per result with title, host, snippet", () => {
        const { container } = render(() => (
            <SearchResults
                node={node([
                    { title: "AgentMux", url: "https://agentmux.ai/docs", snippet: "the docs" },
                    { title: "Two", url: "https://two.com" },
                ])}
            />
        ));
        const cards = container.querySelectorAll(".agent-search-card");
        expect(cards.length).toBe(2);
        expect(container.textContent).toContain("AgentMux");
        expect(container.textContent).toContain("agentmux.ai/docs");
        expect(container.textContent).toContain("the docs");
        expect(container.querySelector(".agent-tool-compact-json")).toBeNull();
    });

    it("opens the URL in the system browser on click", () => {
        const { container } = render(() => (
            <SearchResults node={node([{ title: "X", url: "https://x.com" }])} />
        ));
        fireEvent.click(container.querySelector(".agent-search-card")!);
        expect(openExternal).toHaveBeenCalledWith("https://x.com");
    });

    it("falls back to JSON (CompactResult) when the result isn't search-shaped", () => {
        const { container } = render(() => (
            <SearchResults node={node({ status: "done", count: 3, items: 7 })} />
        ));
        expect(container.querySelector(".agent-search-card")).toBeNull();
        expect(container.querySelector(".agent-tool-compact-result")).not.toBeNull();
    });

    it("is registered for the WebSearch tool by name", () => {
        // Importing this module registered the web:search renderer.
        expect(resolveToolRenderer(node([{ url: "https://a.com" }]))).not.toBeNull();
        // ...and it routes a WebSearch node (toolName) regardless of coarse kind.
        const r = resolveToolRenderer(node([{ url: "https://a.com" }], { tool: "Other" }));
        expect(r).not.toBeNull();
    });
});

// SPEC_TOOL_PREVIEW_CONTENT_FIRST_2026_09_26.md §3.3–3.4 — Claude Code's real
// WebSearch string: the summary first, then a compact sources strip. No
// chevron, no REMINDER, no body header repeating the query.
describe("SearchResults — Claude Code WebSearch text", () => {
    it("renders the summary as markdown, then one source chip per domain", async () => {
        const { container } = render(() => <SearchResults node={node({ content: CLAUDE_WEBSEARCH_RESULT })} />);
        const summary = container.querySelector(".agent-search-summary");
        expect(summary).not.toBeNull();
        await waitFor(() => expect(summary!.textContent).toContain("Official docs"));
        const chips = [...container.querySelectorAll(".agent-search-source")];
        expect(chips.map((c) => c.querySelector(".agent-search-source-domain")!.textContent)).toEqual([
            "docs.solidjs.com",
            "thisdot.co",
            "solidjs.com",
            "youtube.com",
            "vladislav-lipatov.medium.com",
            "raresportan.com",
        ]);
        expect(chips[0].querySelector(".agent-search-source-count")!.textContent).toBe("×2");
        expect(chips[1].querySelector(".agent-search-source-count")).toBeNull();
        // The summary comes before the sources in the DOM.
        expect(summary!.compareDocumentPosition(chips[0]) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    });

    it("has no chevron one-liner, no REMINDER text and no header line", () => {
        const { container } = render(() => <SearchResults node={node({ content: CLAUDE_WEBSEARCH_RESULT })} />);
        expect(container.querySelector(".agent-tool-compact-summary")).toBeNull();
        expect(container.querySelector(".agent-search-header")).toBeNull();
        expect(container.textContent).not.toContain("REMINDER");
    });

    it("a single-link chip opens that link", () => {
        const { container } = render(() => <SearchResults node={node({ content: CLAUDE_WEBSEARCH_RESULT })} />);
        fireEvent.click(container.querySelectorAll(".agent-search-source")[1]);
        expect(openExternal).toHaveBeenCalledWith(
            "https://www.thisdot.co/blog/how-to-handle-async-data-fetching-using-createresource-in-solidjs",
        );
    });

    it("a grouped chip lists its links inline instead of guessing which to open", () => {
        const { container } = render(() => <SearchResults node={node({ content: CLAUDE_WEBSEARCH_RESULT })} />);
        fireEvent.click(container.querySelector(".agent-search-source")!);
        expect(openExternal).not.toHaveBeenCalled();
        const links = [...container.querySelectorAll(".agent-search-link")];
        expect(links.map((l) => l.querySelector(".agent-search-link-title")!.textContent)).toEqual([
            "Fetching data - SolidJS Documentation",
            "createResource - SolidJS Documentation",
        ]);
        fireEvent.click(links[1]);
        expect(openExternal).toHaveBeenCalledWith("https://docs.solidjs.com/reference/basic-reactivity/create-resource");
    });

    it("with links but no summary, lists every link on its own line", () => {
        const text = 'Web search results for query: "q"\n\nLinks: [{"title":"A","url":"https://a.com/x"},{"title":"B","url":"https://b.com"}]';
        const { container } = render(() => <SearchResults node={node(text)} />);
        expect(container.querySelector(".agent-search-summary")).toBeNull();
        expect(container.querySelectorAll(".agent-search-link")).toHaveLength(2);
    });
});
