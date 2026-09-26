// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { ToolNode } from "../types";
import { mcpDisplayName, toolHeaderParts, toolHeaderText } from "./tool-header";

const node = (over: Partial<ToolNode>): ToolNode => ({
    type: "tool",
    id: "t1",
    tool: "Other",
    params: {},
    status: "success",
    collapsed: true,
    summary: "baked summary",
    ...over,
});

describe("toolHeaderParts", () => {
    it("core tools keep their name next to the icon", () => {
        const parts = toolHeaderParts(node({ tool: "Bash", toolName: "Bash", params: { command: "ls -la" } }));
        expect(parts).toEqual({ icon: "🔧", label: "Bash", detail: "ls -la" });
    });

    it("web tools drop the name: the globe plus a query or URL already says what it is", () => {
        const search = toolHeaderParts(node({ toolName: "WebSearch", params: { query: "solid docs" } }));
        expect(search).toEqual({ icon: "🌐", label: null, detail: "solid docs" });
        const fetch = toolHeaderParts(node({ toolName: "WebFetch", params: { url: "https://example.com/a/b" } }));
        expect(fetch).toEqual({ icon: "🌐", label: null, detail: "example.com/a/b" });
    });

    it("a web tool with no detail keeps its name, or the row would be a bare globe", () => {
        expect(toolHeaderParts(node({ toolName: "WebSearch", params: {} })).label).toBe("WebSearch");
    });

    it("reads the detail by the raw tool name, not the coarse kind", () => {
        // WebSearch's coarse kind is "Other", which has no detail rule.
        expect(toolHeaderParts(node({ toolName: "WebSearch", params: { query: "q" } })).detail).toBe("q");
    });

    it("MCP tools show `server · Tool` instead of the raw mcp__ name", () => {
        const parts = toolHeaderParts(node({ toolName: "mcp__agentmux__WhoAmI" }));
        expect(parts).toEqual({ icon: "🛠️", label: "agentmux · WhoAmI", detail: "" });
    });

    it("falls back to the coarse kind for nodes without a raw name", () => {
        const parts = toolHeaderParts(node({ tool: "Read", toolName: undefined, params: { file_path: "a.ts" } }));
        expect(parts).toEqual({ icon: "📖", label: "Read", detail: "a.ts" });
    });
});

describe("toolHeaderText", () => {
    it("joins the parts with single spaces and no status glyph or duration", () => {
        expect(toolHeaderText(node({ toolName: "mcp__agentmux__WhoAmI" }))).toBe("🛠️ agentmux · WhoAmI");
        expect(toolHeaderText(node({ toolName: "WebSearch", params: { query: "solid docs" } }))).toBe("🌐 solid docs");
    });
});

describe("mcpDisplayName", () => {
    it("splits on the first separator after the prefix", () => {
        expect(mcpDisplayName("mcp__agentmux__WhoAmI")).toBe("agentmux · WhoAmI");
        expect(mcpDisplayName("mcp__claude_ai_Docs__read")).toBe("claude_ai_Docs · read");
        expect(mcpDisplayName("mcp__github__search__issues")).toBe("github · search__issues");
    });

    it("returns null for anything that is not a well-formed MCP name", () => {
        expect(mcpDisplayName("WebSearch")).toBeNull();
        expect(mcpDisplayName("mcp__noseparator")).toBeNull();
        expect(mcpDisplayName("mcp____x")).toBeNull();
    });
});
