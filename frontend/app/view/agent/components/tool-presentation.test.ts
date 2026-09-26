// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { ToolNode } from "../types";
import { isContentFirstTool, startsAtTop } from "./tool-presentation";

const node = (toolName: string, status: ToolNode["status"]): ToolNode => ({
    type: "tool",
    id: "t1",
    tool: toolName === "Bash" ? "Bash" : "Other",
    toolName,
    params: {},
    status,
    collapsed: true,
    summary: "x",
});

describe("isContentFirstTool", () => {
    it("is true for a finished WebSearch (success or failed), by either provider name", () => {
        expect(isContentFirstTool(node("WebSearch", "success"))).toBe(true);
        expect(isContentFirstTool(node("WebSearch", "failed"))).toBe(true);
        expect(isContentFirstTool(node("web_search", "success"))).toBe(true);
    });

    it("is false while active: a running tool is already auto-expanded", () => {
        expect(isContentFirstTool(node("WebSearch", "running"))).toBe(false);
        expect(isContentFirstTool(node("WebSearch", "pending_approval"))).toBe(false);
    });

    it("is false for dismissed tools, which have no content to show", () => {
        expect(isContentFirstTool(node("WebSearch", "canceled"))).toBe(false);
        expect(isContentFirstTool(node("WebSearch", "denied"))).toBe(false);
    });

    it("is false for every other tool", () => {
        expect(isContentFirstTool(node("Bash", "success"))).toBe(false);
        expect(isContentFirstTool(node("WebFetch", "success"))).toBe(false);
    });
});

describe("startsAtTop", () => {
    it("is true for content-first tools at any status, so the result opens at its start", () => {
        expect(startsAtTop(node("WebSearch", "running"))).toBe(true);
        expect(startsAtTop(node("WebSearch", "success"))).toBe(true);
        expect(startsAtTop(node("Bash", "success"))).toBe(false);
    });
});
