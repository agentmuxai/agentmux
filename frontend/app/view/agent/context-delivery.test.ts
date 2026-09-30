// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    CompactionSummaryTracker,
    compactionSummaryExcerpt,
    compactionSummaryNodeId,
    contextDeliveryTitle,
} from "./context-delivery";
import { contextCompactedNodeId, type CompactBoundaryData } from "./compact-boundary";

const SUMMARY = [
    "This session is being continued from a previous conversation that ran out of context. The summary below covers the earlier portion of the conversation.",
    "",
    "Summary:",
    "1. Primary Request and Intent:",
    "",
    "   The user asked to make the Cloud Console sign in with the new pool's client, then to make that durable.",
    "",
    "2. Key Technical Concepts:",
    "   - Cognito PKCE",
].join("\n");

const boundary: CompactBoundaryData = {
    trigger: "manual",
    preTokens: 120_000,
    postTokens: 8_000,
    durationMs: 30_000,
    frameTimestamp: "2026-09-30T08:00:00.000Z",
};

const summaryFrame = (content: unknown = SUMMARY, extra: Record<string, unknown> = { isSynthetic: true }) => ({
    type: "user",
    message: { role: "user", content },
    ...extra,
});

describe("CompactionSummaryTracker", () => {
    it("turns the summary frame after a boundary into one context delivery", () => {
        const t = new CompactionSummaryTracker();
        t.noteBoundary(boundary);
        const node = t.take(summaryFrame(), 1_000);
        expect(node).not.toBeNull();
        expect(node!.type).toBe("context_delivery");
        expect(node!.reason).toBe("compaction");
        expect(node!.trigger).toBe("manual");
        expect(node!.id).toBe(compactionSummaryNodeId(summaryFrame(), boundary, SUMMARY));
        expect(node!.timestamp).toBe(Date.parse(boundary.frameTimestamp!));
        expect(node!.items).toHaveLength(1);
        const item = node!.items[0];
        expect(item.kind).toBe("compaction_summary");
        expect(item.body).toBe(SUMMARY);
        expect(item.sizeBytes).toBe(new TextEncoder().encode(SUMMARY).length);
        expect(item.tokens).toBeGreaterThan(0);
        expect(item.excerpt).toMatch(/^The user asked to make the Cloud Console sign in/);
    });

    it("accepts the text signal alone (no isSynthetic flag)", () => {
        const t = new CompactionSummaryTracker();
        t.noteBoundary(boundary);
        expect(t.take(summaryFrame(SUMMARY, {}), 0)).not.toBeNull();
    });

    it("accepts the flag alone (reworded summary text)", () => {
        const t = new CompactionSummaryTracker();
        t.noteBoundary(boundary);
        expect(t.take(summaryFrame("Earlier conversation, summarized:\n1. Intent: fix sign-in"), 0)).not.toBeNull();
    });

    it("without a boundary in view (a split history page), needs both signals", () => {
        const t = new CompactionSummaryTracker();
        const node = t.take(summaryFrame(), 0);
        expect(node).not.toBeNull();
        expect(node!.trigger).toBeUndefined();
        expect(t.take(summaryFrame(SUMMARY, {}), 0)).toBeNull();
        expect(t.take(summaryFrame("Earlier conversation, summarized."), 0)).toBeNull();
    });

    it("prefers the frame's own timestamp", () => {
        const t = new CompactionSummaryTracker();
        t.noteBoundary(boundary);
        const node = t.take(summaryFrame(SUMMARY, { isSynthetic: true, timestamp: "2026-09-30T08:00:05.000Z" }), 0);
        expect(node!.timestamp).toBe(Date.parse("2026-09-30T08:00:05.000Z"));
    });

    it("leaves a plain user message after a boundary alone", () => {
        const t = new CompactionSummaryTracker();
        t.noteBoundary(boundary);
        expect(t.take(summaryFrame("please continue", {}), 0)).toBeNull();
    });

    it("pairs a boundary with at most one summary", () => {
        const t = new CompactionSummaryTracker();
        t.noteBoundary(boundary);
        expect(t.take(summaryFrame(), 0)).not.toBeNull();
        // One signal only: accepted solely right after a boundary, which is spent.
        expect(t.take(summaryFrame(SUMMARY, {}), 0)).toBeNull();
    });

    it("forgets the boundary once the assistant speaks", () => {
        const t = new CompactionSummaryTracker();
        t.noteBoundary(boundary);
        expect(t.take({ type: "assistant", message: { content: [] } }, 0)).toBeNull();
        expect(t.take(summaryFrame(SUMMARY, {}), 0)).toBeNull();
    });

    it("keeps waiting across frames that aren't user or assistant turns", () => {
        const t = new CompactionSummaryTracker();
        t.noteBoundary(boundary);
        expect(t.take({ type: "system", subtype: "status" }, 0)).toBeNull();
        expect(t.take(summaryFrame(), 0)).not.toBeNull();
    });

    it("ignores array-content user frames (tool results) without forgetting the boundary", () => {
        const t = new CompactionSummaryTracker();
        t.noteBoundary(boundary);
        expect(t.take(summaryFrame([{ type: "tool_result", tool_use_id: "x", content: "ok" }]), 0)).toBeNull();
        expect(t.take(summaryFrame(), 0)).not.toBeNull();
    });

    it("falls back to the caller's time when the boundary has no timestamp", () => {
        const t = new CompactionSummaryTracker();
        t.noteBoundary({ ...boundary, frameTimestamp: null });
        expect(t.take(summaryFrame(), 42)!.timestamp).toBe(42);
    });
});

describe("contextDeliveryTitle", () => {
    const summaryNode = (trigger?: "manual" | "auto") => {
        const t = new CompactionSummaryTracker();
        t.noteBoundary({ ...boundary, trigger: trigger as any });
        return t.take(summaryFrame(), 0)!;
    };

    it("names the compaction summary and how it was triggered", () => {
        expect(contextDeliveryTitle(summaryNode("manual"))).toBe("Agent given a summary of the conversation (manual compact)");
        expect(contextDeliveryTitle(summaryNode("auto"))).toBe("Agent given a summary of the conversation (auto-compact)");
    });

    it("counts items for other deliveries", () => {
        const node = { ...summaryNode("manual"), reason: "clear" as const };
        node.items = [
            { kind: "global_memory", name: "App API", sizeBytes: 10, tokens: 3 },
            { kind: "personal_memory", name: "MEMORY.md", sizeBytes: 10, tokens: 3 },
        ];
        expect(contextDeliveryTitle(node)).toBe("Given to the agent · after /clear · 2 items");
    });
});

describe("compactionSummaryNodeId", () => {
    it("uses the frame's uuid, so live and every replayed page agree", () => {
        const frame = summaryFrame(SUMMARY, { isSynthetic: true, uuid: "u-1" });
        expect(compactionSummaryNodeId(frame, boundary, SUMMARY)).toBe(compactionSummaryNodeId(frame, null, SUMMARY));
    });

    it("falls back to the boundary, distinct from the compacted row's id", () => {
        const id = compactionSummaryNodeId(summaryFrame(), boundary, SUMMARY);
        expect(id).toBe(compactionSummaryNodeId(summaryFrame(), { ...boundary }, SUMMARY));
        expect(id).not.toBe(contextCompactedNodeId(boundary));
    });
});

describe("compactionSummaryExcerpt", () => {
    it("takes the first line of real content after the preamble and headings", () => {
        expect(compactionSummaryExcerpt(SUMMARY)).toMatch(/^The user asked/);
    });

    it("caps the excerpt at 160 characters", () => {
        const long = `This session is being continued.\n\nSummary:\n${"word ".repeat(100)}`;
        const excerpt = compactionSummaryExcerpt(long);
        expect(excerpt.length).toBeLessThanOrEqual(160);
        expect(excerpt.endsWith("…")).toBe(true);
    });

    it("returns an empty string when there's nothing but the preamble", () => {
        expect(compactionSummaryExcerpt("This session is being continued from a previous conversation.")).toBe("");
    });
});
