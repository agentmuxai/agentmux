// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { DocumentNode, ToolNode } from "../types";
import { isToolPromoted, nextToolPromotionAt, TOOL_PROMOTION_MS, toolActivities, toolToActivity } from "./tool-adapter";

function mkBash(overrides: Partial<ToolNode> = {}): ToolNode {
    return {
        type: "tool",
        id: "tool-1",
        tool: "Bash",
        status: "running",
        params: { command: "sleep 300" },
        collapsed: false,
        summary: "",
        timestamp: 0,
        ...overrides,
    };
}

describe("toolToActivity", () => {
    it("maps a running Bash node to a running, non-stoppable activity", () => {
        const n = mkBash({ id: "t1", timestamp: 1000, params: { command: "sleep 300" } });
        const a = toolToActivity(n);
        expect(a.id).toBe("t1");
        expect(a.kind).toBe("tool");
        expect(a.status).toBe("running");
        expect(a.startedAt).toBe(1000);
        expect(a.canStop).toBe(false);
        expect(a.title).toBe("sleep 300");
        expect(a.tool).toBe(n);
    });

    it("falls back to the tool name when params carry no command text", () => {
        const n = mkBash({ params: {}, toolName: "Bash" });
        expect(toolToActivity(n).title).toBe("Bash");
    });
});

describe("toolActivities", () => {
    it("excludes a Bash call that hasn't crossed the promotion threshold yet", () => {
        const nodes: DocumentNode[] = [mkBash({ id: "t1", timestamp: 1000 })];
        expect(toolActivities(nodes, 1000 + TOOL_PROMOTION_MS - 1)).toEqual([]);
    });

    it("includes a Bash call exactly at and past the promotion threshold", () => {
        const nodes: DocumentNode[] = [mkBash({ id: "t1", timestamp: 1000 })];
        expect(toolActivities(nodes, 1000 + TOOL_PROMOTION_MS).map((a) => a.id)).toEqual(["t1"]);
        expect(toolActivities(nodes, 1000 + TOOL_PROMOTION_MS + 5000).map((a) => a.id)).toEqual(["t1"]);
    });

    it("never promotes a non-Bash tool call, regardless of duration", () => {
        const nodes: DocumentNode[] = [mkBash({ id: "t1", tool: "Read", timestamp: 0 })];
        expect(toolActivities(nodes, 1_000_000)).toEqual([]);
    });

    it("never promotes a Bash call that already finished, regardless of duration", () => {
        const nodes: DocumentNode[] = [mkBash({ id: "t1", status: "success", timestamp: 0 })];
        expect(toolActivities(nodes, 1_000_000)).toEqual([]);
    });

    it("ignores nodes with no timestamp (pre-field-add back-compat)", () => {
        const nodes: DocumentNode[] = [mkBash({ id: "t1", timestamp: undefined })];
        expect(toolActivities(nodes, 1_000_000)).toEqual([]);
    });
});

describe("nextToolPromotionAt", () => {
    it("returns null when nothing is pending promotion", () => {
        expect(nextToolPromotionAt([], 0)).toBeNull();
    });

    it("returns the promotion instant of a still-running, not-yet-promoted call", () => {
        const nodes: DocumentNode[] = [mkBash({ timestamp: 1000 })];
        expect(nextToolPromotionAt(nodes, 1000)).toBe(1000 + TOOL_PROMOTION_MS);
    });

    it("returns null once the call has already crossed the threshold (nothing left to schedule)", () => {
        const nodes: DocumentNode[] = [mkBash({ timestamp: 1000 })];
        expect(nextToolPromotionAt(nodes, 1000 + TOOL_PROMOTION_MS)).toBeNull();
    });

    it("picks the earliest pending promotion among several running calls", () => {
        const nodes: DocumentNode[] = [
            mkBash({ id: "t1", timestamp: 5000 }),
            mkBash({ id: "t2", timestamp: 1000 }),
        ];
        expect(nextToolPromotionAt(nodes, 1000)).toBe(1000 + TOOL_PROMOTION_MS);
    });
});

describe("isToolPromoted", () => {
    it("is false for a null id", () => {
        expect(isToolPromoted([mkBash()], null, 1_000_000)).toBe(false);
    });

    it("is false before the threshold and true at/after it", () => {
        const nodes: DocumentNode[] = [mkBash({ id: "t1", timestamp: 1000 })];
        expect(isToolPromoted(nodes, "t1", 1000 + TOOL_PROMOTION_MS - 1)).toBe(false);
        expect(isToolPromoted(nodes, "t1", 1000 + TOOL_PROMOTION_MS)).toBe(true);
    });

    it("is false when the id doesn't match any node", () => {
        const nodes: DocumentNode[] = [mkBash({ id: "t1", timestamp: 0 })];
        expect(isToolPromoted(nodes, "missing", 1_000_000)).toBe(false);
    });
});
