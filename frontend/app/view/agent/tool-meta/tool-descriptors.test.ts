// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { ToolNode } from "../types";
import {
    TOOL_DESCRIPTORS,
    isContentFirstTool,
    resolveFact,
    startsAtTop,
    toolDetail,
    toolDetailOf,
    toolIcon,
    toolNameOf,
    type ToolDescriptor,
} from "./tool-descriptors";

const node = (toolName: string | undefined, tool: ToolNode["tool"] = "Other"): ToolNode => ({
    type: "tool",
    id: "t",
    tool,
    toolName,
    params: {},
    status: "success",
    collapsed: true,
    summary: "",
});

describe("resolveFact", () => {
    const table: ToolDescriptor[] = [
        { names: ["Exact"], icon: "E" },
        { prefix: "mcp__", label: () => "prefixed" },
        { names: ["mcp__x__named"], icon: "N" },
        { icon: "D", label: (name) => name },
    ];

    it("takes the first matching descriptor that defines the field, field by field", () => {
        // The prefix supplies the label, the catch-all the icon.
        expect(resolveFact(table, "mcp__srv__tool", "label")?.("mcp__srv__tool", "")).toBe("prefixed");
        expect(resolveFact(table, "mcp__srv__tool", "icon")).toBe("D");
    });

    it("prefers a names match over a prefix match, wherever it sits in the table", () => {
        expect(resolveFact(table, "mcp__x__named", "icon")).toBe("N");
        expect(resolveFact(table, "mcp__x__named", "label")?.("mcp__x__named", "")).toBe("prefixed");
    });

    it("falls back to the catch-all for unknown names", () => {
        expect(resolveFact(table, "Unknown", "icon")).toBe("D");
        expect(resolveFact(table, "Exact", "icon")).toBe("E");
    });
});

describe("TOOL_DESCRIPTORS", () => {
    it("ends with a catch-all that supplies an icon and a label", () => {
        const last = TOOL_DESCRIPTORS[TOOL_DESCRIPTORS.length - 1];
        expect(last.names).toBeUndefined();
        expect(last.prefix).toBeUndefined();
        expect(last.icon).toBeTruthy();
        expect(last.label).toBeTypeOf("function");
    });

    it("names each raw name at most once", () => {
        const seen = new Map<string, number>();
        for (const d of TOOL_DESCRIPTORS) for (const n of d.names ?? []) seen.set(n, (seen.get(n) ?? 0) + 1);
        expect([...seen].filter(([, n]) => n > 1)).toEqual([]);
    });
});

describe("helpers", () => {
    it("toolNameOf prefers the raw name, falling back to the coarse kind", () => {
        expect(toolNameOf(node("WebSearch"))).toBe("WebSearch");
        expect(toolNameOf(node(undefined, "Read"))).toBe("Read");
    });

    it("toolIcon resolves by raw name, then the catch-all", () => {
        expect(toolIcon(node("Read", "Read"))).toBe("📖");
        expect(toolIcon(node(undefined, "Bash"))).toBe("🔧");
        expect(toolIcon(node("SomethingNew"))).toBe("🛠️");
    });

    it("toolDetail has no generic fallback: unknown tools have no header detail", () => {
        expect(toolDetail("mcp__github__search_issues", { query: "bug" })).toBe("");
        expect(toolDetail("Grep", { pattern: "x" })).toBe("x");
    });
});

// Codex P2 on #3901: normalizeToolName() maps a noncanonical casing ("READ",
// "bAsH") to its coarse kind, and the descriptors must too, or the row loses
// its kind's icon, scroll start and detail.
describe("coarse-kind fallback", () => {
    const odd = (toolName: string, tool: ToolNode["tool"], params: Record<string, any>): ToolNode => ({
        ...node(toolName, tool),
        params,
    });

    it("a raw name with no descriptor of its own resolves through its coarse kind", () => {
        expect(toolIcon(odd("READ", "Read", {}))).toBe("📖");
        expect(startsAtTop(odd("READ", "Read", {}))).toBe(true);
        expect(toolDetailOf(odd("bAsH", "Bash", { command: "ls" }))).toBe("ls");
    });

    it("a raw name's own descriptor still wins over its coarse kind", () => {
        // WebSearch's coarse kind is "Other"; its own facts apply.
        expect(isContentFirstTool({ ...odd("WebSearch", "Other", {}), status: "success" })).toBe(true);
        expect(toolIcon(odd("WebSearch", "Other", {}))).toBe("🌐");
    });
});
