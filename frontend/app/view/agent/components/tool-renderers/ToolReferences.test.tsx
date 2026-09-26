// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import type { ToolNode } from "../../types";
import { resolveToolRenderer } from "./registry";
// Registers the built-ins (the catch-all) alongside ToolReferences' own entry.
import "../ToolOverlayLog";
import { ToolReferences } from "./ToolReferences";

afterEach(() => cleanup());

// A real ToolSearch result (captured from a transcript on 2026-09-26): the
// translator passes tool_reference blocks through, and they used to render as
// a `type | tool_name` RecordTable. SPEC_TOOL_PREVIEW_CONTENT_FIRST_2026_09_26.md §3.7.
const node = (result: unknown): ToolNode => ({
    type: "tool",
    id: "ts1",
    tool: "Other",
    toolName: "ToolSearch",
    params: { query: "select:WebSearch,mcp__agentmux__SendMessage" },
    status: "success",
    collapsed: true,
    summary: "x",
    result: result as any,
});

describe("ToolReferences", () => {
    it("renders one chip per loaded tool, MCP names as `server · Tool`", () => {
        const { container } = render(() => (
            <ToolReferences
                node={node([
                    { type: "tool_reference", tool_name: "WebSearch" },
                    { type: "tool_reference", tool_name: "mcp__agentmux__SendMessage" },
                ])}
            />
        ));
        const chips = [...container.querySelectorAll(".agent-tool-reference")].map((c) => c.textContent);
        expect(chips).toEqual(["WebSearch", "agentmux · SendMessage"]);
        expect(container.querySelector(".agent-tool-record-table")).toBeNull();
    });

    it("falls back to the default body for anything else", () => {
        const { container } = render(() => <ToolReferences node={node({ content: "No matching deferred tools" })} />);
        expect(container.querySelector(".agent-tool-reference")).toBeNull();
        expect(container.textContent).toContain("No matching deferred tools");
    });

    it("is registered for ToolSearch by name", () => {
        const r = resolveToolRenderer(node([{ type: "tool_reference", tool_name: "WebSearch" }]))!;
        const { container } = render(() => r(node([{ type: "tool_reference", tool_name: "WebSearch" }])));
        expect(container.querySelector(".agent-tool-reference")).not.toBeNull();
    });
});
