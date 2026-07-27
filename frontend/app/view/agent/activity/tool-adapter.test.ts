// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { DocumentNode, ToolNode } from "../types";
import { hasRunningPromotedTool, nextToolPromotionAt, TOOL_PROMOTION_MS, toolActivities, toolToActivity } from "./tool-adapter";

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
        expect(a.endedAt).toBeUndefined();
        expect(a.canStop).toBe(false);
        expect(a.title).toBe("sleep 300");
        expect(a.tool).toBe(n);
    });

    it("falls back to the tool name when params carry no command text", () => {
        const n = mkBash({ params: {}, toolName: "Bash" });
        expect(toolToActivity(n).title).toBe("Bash");
    });

    it("maps a finished call to done/error with endedAt = timestamp + duration", () => {
        const done = toolToActivity(mkBash({ status: "success", timestamp: 1000, duration: 45 }));
        expect(done.status).toBe("done");
        expect(done.endedAt).toBe(1000 + 45_000);

        const err = toolToActivity(mkBash({ status: "failed", timestamp: 1000, duration: 45 }));
        expect(err.status).toBe("error");
        expect(err.endedAt).toBe(1000 + 45_000);
    });

    it("maps denied/canceled to stopped — cut off, not a failure signal", () => {
        expect(toolToActivity(mkBash({ status: "denied" })).status).toBe("stopped");
        expect(toolToActivity(mkBash({ status: "canceled" })).status).toBe("stopped");
    });
});

describe("toolActivities", () => {
    it("excludes a running Bash call that hasn't crossed the promotion threshold yet", () => {
        const nodes: DocumentNode[] = [mkBash({ id: "t1", timestamp: 1000 })];
        expect(toolActivities(nodes, 1000 + TOOL_PROMOTION_MS - 1)).toEqual([]);
    });

    it("includes a running Bash call exactly at and past the promotion threshold", () => {
        const nodes: DocumentNode[] = [mkBash({ id: "t1", timestamp: 1000 })];
        expect(toolActivities(nodes, 1000 + TOOL_PROMOTION_MS).map((a) => a.id)).toEqual(["t1"]);
        expect(toolActivities(nodes, 1000 + TOOL_PROMOTION_MS + 5000).map((a) => a.id)).toEqual(["t1"]);
    });

    it("never promotes a non-Bash tool call, regardless of duration", () => {
        const nodes: DocumentNode[] = [mkBash({ id: "t1", tool: "Read", timestamp: 0 })];
        expect(toolActivities(nodes, 1_000_000)).toEqual([]);
    });

    it("ignores nodes with no timestamp (pre-field-add back-compat)", () => {
        const nodes: DocumentNode[] = [mkBash({ id: "t1", timestamp: undefined })];
        expect(toolActivities(nodes, 1_000_000)).toEqual([]);
    });

    it("keeps a finished call whose total duration crossed the threshold — inherits the dock's normal retention lifecycle instead of vanishing on ToolEnd", () => {
        const nodes: DocumentNode[] = [mkBash({ id: "t1", status: "success", timestamp: 1000, duration: 40 })];
        // Long after it finished — still returned; ActivityDock's own
        // RETENTION_MS + endedAt filtering, not this function, decides
        // when the row actually disappears from view.
        const result = toolActivities(nodes, 1000 + 40_000 + 60_000);
        expect(result.map((a) => a.id)).toEqual(["t1"]);
        expect(result[0].status).toBe("done");
    });

    it("never promotes a call that finished before crossing the threshold", () => {
        const nodes: DocumentNode[] = [mkBash({ id: "t1", status: "success", timestamp: 1000, duration: 5 })];
        expect(toolActivities(nodes, 1000 + 5000)).toEqual([]);
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

    it("ignores a finished call — nothing left to schedule for it", () => {
        const nodes: DocumentNode[] = [mkBash({ status: "success", timestamp: 1000, duration: 40 })];
        expect(nextToolPromotionAt(nodes, 2000)).toBeNull();
    });

    it("moves on to the next-earliest pending call once the first is excluded (simulates a reschedule after the first fires)", () => {
        const nodes: DocumentNode[] = [
            mkBash({ id: "t1", timestamp: 1000 }),
            mkBash({ id: "t2", timestamp: 5000 }),
        ];
        // At the instant t1 crosses the threshold, a re-run should now
        // surface t2's own promotion instant instead of returning null.
        expect(nextToolPromotionAt(nodes, 1000 + TOOL_PROMOTION_MS)).toBe(5000 + TOOL_PROMOTION_MS);
    });
});

describe("hasRunningPromotedTool", () => {
    it("is false before the threshold and true at/after it, for a running call", () => {
        const nodes: DocumentNode[] = [mkBash({ id: "t1", timestamp: 1000 })];
        expect(hasRunningPromotedTool(nodes, 1000 + TOOL_PROMOTION_MS - 1)).toBe(false);
        expect(hasRunningPromotedTool(nodes, 1000 + TOOL_PROMOTION_MS)).toBe(true);
    });

    it("is false for a finished call still lingering in the dock's retention window — must not suppress a different, newly-started tool's working-row text", () => {
        const nodes: DocumentNode[] = [mkBash({ id: "t1", status: "success", timestamp: 1000, duration: 40 })];
        expect(hasRunningPromotedTool(nodes, 1000 + 40_000 + 1000)).toBe(false);
    });

    it("is false with no nodes", () => {
        expect(hasRunningPromotedTool([], 1_000_000)).toBe(false);
    });
});
