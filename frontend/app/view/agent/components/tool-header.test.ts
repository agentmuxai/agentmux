// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { ToolNode } from "../types";
import { mcpDisplayName } from "../tool-meta/tool-descriptors";
import { toolHeaderParts, toolHeaderText } from "./tool-header";

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
    // REPORT_THEME_MENU_RUNTIME_PANEL_TOOL_PREVIEW_TWEAKS_2026_10_07.md §4: a file
    // path is cut from the left in the row, so the header says which details are paths.
    it("marks a Read, Write or Edit path as a path, and nothing else", () => {
        for (const toolName of ["Read", "Write", "Edit", "multiedit", "str_replace_editor"]) {
            expect(toolHeaderParts(node({ toolName, params: { file_path: "/a/b/c.ts" } })).detailIsPath, toolName).toBe(true);
        }
        expect(toolHeaderParts(node({ tool: "Bash", toolName: "Bash", params: { command: "cat /a/b" } })).detailIsPath).toBe(false);
        expect(toolHeaderParts(node({ toolName: "Grep", params: { pattern: "x", path: "/a" } })).detailIsPath).toBe(false);
        // No path, no path layout.
        expect(toolHeaderParts(node({ toolName: "Read", params: {} })).detailIsPath).toBe(false);
    });

    it("core tools keep their name next to the icon", () => {
        const parts = toolHeaderParts(node({ tool: "Bash", toolName: "Bash", params: { command: "ls -la" } }));
        expect(parts).toEqual({ icon: "🔧", label: "Bash", detail: "ls -la", detailIsPath: false, range: null });
    });

    it("web tools drop the name: the globe plus a query or URL already says what it is", () => {
        const search = toolHeaderParts(node({ toolName: "WebSearch", params: { query: "solid docs" } }));
        expect(search).toEqual({ icon: "🌐", label: null, detail: "solid docs", detailIsPath: false, range: null });
        const fetch = toolHeaderParts(node({ toolName: "WebFetch", params: { url: "https://example.com/a/b" } }));
        expect(fetch).toEqual({ icon: "🌐", label: null, detail: "example.com/a/b", detailIsPath: false, range: null });
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
        expect(parts).toEqual({ icon: "🛠️", label: "agentmux · WhoAmI", detail: "", detailIsPath: false, range: null });
    });

    it("falls back to the coarse kind for nodes without a raw name", () => {
        const parts = toolHeaderParts(node({ tool: "Read", toolName: undefined, params: { file_path: "a.ts" } }));
        expect(parts).toEqual({ icon: "📖", label: "Read", detail: "a.ts", detailIsPath: true, range: null });
    });
});

describe("toolHeaderText", () => {
    it("appends a reducer-set status note so text consumers keep it", () => {
        expect(toolHeaderText(node({ tool: "Bash", toolName: "Bash", params: { command: "ls" }, statusNote: "cleared via muxspect" }))).toBe(
            "🔧 Bash ls — cleared via muxspect",
        );
    });

    it("joins the parts with single spaces and no status glyph or duration", () => {
        expect(toolHeaderText(node({ toolName: "mcp__agentmux__WhoAmI" }))).toBe("🛠️ agentmux · WhoAmI");
        expect(toolHeaderText(node({ toolName: "WebSearch", params: { query: "solid docs" } }))).toBe("🌐 solid docs");
    });
});

describe("a Read's range in the header", () => {
    it("is its own part, and ends the plain-text form", () => {
        const read = node({ tool: "Read", toolName: "Read", params: { file_path: "a.ts", offset: 5, limit: 3 } });
        expect(toolHeaderParts(read)).toEqual({ icon: "📖", label: "Read", detail: "a.ts", detailIsPath: true, range: "5:7" });
        expect(toolHeaderText(read)).toBe("📖 Read 5:7 a.ts");
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
