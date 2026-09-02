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
    /**
     * Liveness bound, NOT a performance assertion — see the timeout argument
     * at the end of this `it`.
     *
     * This test flaked twice on 2026-08-31/09-01, failing at exactly 5005ms
     * (vitest's default per-test timeout) on branches that couldn't have
     * caused it. It is not marginally slow, it is a genuine outlier: measured
     * across a full local run of 3500 tests it takes **4.32s**, while the
     * next-slowest test in the entire suite is 2.02s and only four tests
     * exceed 1s at all. So on an idle machine it already sits 86% of the way
     * to the ceiling; a loaded CI runner tips it over.
     *
     * The cost is the `await import("../ToolOverlayLog")` below, which drags
     * the whole renderer tree through vitest's transform. That import IS the
     * thing under test (the registrations are its module side effect), so it
     * can't be stubbed away without deleting the coverage.
     *
     * Deliberately a per-test timeout rather than a global `testTimeout` bump
     * in vitest.config.ts: the distribution above shows this is one unusually
     * expensive test, not a suite-wide margin problem. Raising the default for
     * all 3500 would slow the detection of genuine hangs everywhere to fix a
     * problem that exists in exactly one place. Issue #2919.
     */
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
        // 30s, ~7× the 4.32s measured worst case. Costs nothing on the happy
        // path; if this test ever genuinely takes 30s something is wedged and
        // the failure is real. Same reasoning as IO_TIMEOUT in
        // agentmux-srv/tests/subprocess_io.rs (issue #2863 / PR #2911).
    }, 30_000);
});
