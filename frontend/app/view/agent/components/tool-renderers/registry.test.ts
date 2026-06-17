// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    resolveFrom,
    byKind,
    byName,
    byNamePrefix,
    byShape,
    anyTool,
    toolNameOf,
    type ToolRendererEntry,
} from "./registry";
import type { ToolNode } from "../../types";

const tool = (over: Partial<ToolNode> = {}): ToolNode => ({
    type: "tool",
    id: "t1",
    tool: "Other",
    params: {},
    status: "success",
    collapsed: true,
    summary: "x",
    ...over,
});

const entry = (label: string, priority: number, match: ToolRendererEntry["match"]): ToolRendererEntry => ({
    label,
    priority,
    match,
    render: () => label as any, // identity-by-label stub
});

describe("tool-renderer registry — resolution", () => {
    it("picks the highest-priority matching entry", () => {
        const list = [
            entry("default", -Infinity, anyTool),
            entry("kind", 0, byKind("Bash")),
            entry("name", 10, byName("Bash")),
        ];
        const r = resolveFrom(list, tool({ tool: "Bash", toolName: "Bash" }));
        expect(r?.(tool())).toBe("name");
    });

    it("breaks ties by registration order (earliest wins)", () => {
        const list = [entry("first", 5, anyTool), entry("second", 5, anyTool)];
        expect(resolveFrom(list, tool())?.(tool())).toBe("first");
    });

    it("falls through to the catch-all when only anyTool matches", () => {
        const list = [entry("kind", 0, byKind("Read")), entry("default", -Infinity, anyTool)];
        expect(resolveFrom(list, tool({ tool: "Other" }))?.(tool())).toBe("default");
    });

    it("returns null when nothing matches", () => {
        const list = [entry("kind", 0, byKind("Read"))];
        expect(resolveFrom(list, tool({ tool: "Other" }))).toBeNull();
    });
});

describe("tool-renderer registry — matchers", () => {
    it("byKind matches the coarse kind", () => {
        expect(byKind("Grep", "Glob")(tool({ tool: "Glob" }))).toBe(true);
        expect(byKind("Grep", "Glob")(tool({ tool: "Bash" }))).toBe(false);
    });

    it("byName matches the raw provider name, falling back to kind", () => {
        expect(byName("WebSearch")(tool({ tool: "Other", toolName: "WebSearch" }))).toBe(true);
        // no toolName → falls back to the coarse kind
        expect(byName("Bash")(tool({ tool: "Bash" }))).toBe(true);
        expect(byName("WebSearch")(tool({ tool: "Other" }))).toBe(false);
    });

    it("byNamePrefix matches mcp__* tools", () => {
        expect(byNamePrefix("mcp__")(tool({ tool: "Other", toolName: "mcp__gh__search" }))).toBe(true);
        expect(byNamePrefix("mcp__")(tool({ tool: "Other", toolName: "WebSearch" }))).toBe(false);
    });

    it("byShape matches on the result", () => {
        const hasResults = (r: unknown): boolean => !!r && typeof r === "object" && Array.isArray((r as any).results);
        expect(byShape(hasResults)(tool({ result: { results: [1] } as any }))).toBe(true);
        expect(byShape(hasResults)(tool({ result: { other: 1 } as any }))).toBe(false);
    });

    it("toolNameOf prefers toolName, falls back to kind", () => {
        expect(toolNameOf(tool({ tool: "Other", toolName: "WebSearch" }))).toBe("WebSearch");
        expect(toolNameOf(tool({ tool: "Bash" }))).toBe("Bash");
    });
});

describe("tool-renderer registry — built-ins (parity)", () => {
    it("registers all built-in renderers and routes every kind through the registry", async () => {
        // Importing ToolOverlayLog runs the built-in registrations (module side
        // effect). Assert the built-ins are present and resolution is total.
        const { _registeredLabels, resolveToolRenderer } = await import("./registry");
        await import("../ToolOverlayLog");
        const labels = _registeredLabels();
        for (const l of [
            "builtin:Edit",
            "builtin:Bash",
            "builtin:Read",
            "builtin:Write",
            "builtin:Search",
            "builtin:Agent",
            "builtin:Task",
            "builtin:default",
        ]) {
            expect(labels).toContain(l);
        }
        // Every kind (and an unknown tool) resolves to *some* renderer.
        for (const k of ["Edit", "Bash", "Read", "Write", "Grep", "Glob", "Agent", "Task", "Other"] as const) {
            expect(resolveToolRenderer(tool({ tool: k }))).not.toBeNull();
        }
        // An unknown provider tool (no built-in kind) still resolves (catch-all).
        expect(resolveToolRenderer(tool({ tool: "Other", toolName: "WebSearch" }))).not.toBeNull();
    });
});
