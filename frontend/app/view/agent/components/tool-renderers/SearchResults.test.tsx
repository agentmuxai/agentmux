// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const openExternal = vi.fn();
vi.mock("@/store/global", () => ({ getApi: () => ({ openExternal }) }));

import { SearchResults } from "./SearchResults";
import { resolveToolRenderer } from "./registry";
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
