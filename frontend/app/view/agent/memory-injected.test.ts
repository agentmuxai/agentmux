// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { buildMemoryInjectedNode, isMemoryInjectedFrame } from "./memory-injected";
import { parseHistoryLines } from "./parseHistoryLines";

// The frame `server/memory_delivery_handlers.rs` `notice_frame` writes.
const frame = {
    type: "system",
    subtype: "agentmux_memory_injected",
    id: "memory-injected-s1-startup-1790000000000",
    reason: "startup",
    session_id: "s1",
    parts: 2,
    entries: [
        { label: "[AgentMux System] App API", source: "global", size_bytes: 3400, tokens: 850 },
        { label: "[Workspace] Rules", source: "global", size_bytes: 1000, tokens: 250 },
        { label: "notes.md", source: "personal", size_bytes: 400, tokens: 100 },
    ],
    summary_bytes: 0,
    timestamp: "2026-09-27T08:00:00+00:00",
};

describe("buildMemoryInjectedNode", () => {
    it("recognises only the memory-injected frame", () => {
        expect(isMemoryInjectedFrame(frame)).toBe(true);
        expect(isMemoryInjectedFrame({ type: "system", subtype: "agentmux_session_outcome" })).toBe(false);
        expect(buildMemoryInjectedNode({ type: "assistant" }, { now: 0 })).toBeNull();
    });

    it("builds the notice from the frame: counts, per-entry sizes, totals (D11)", () => {
        const node = buildMemoryInjectedNode(frame, { contextWindow: 200_000, now: 0 })!;
        expect(node.type).toBe("memory_reinjection");
        expect(node.id).toBe(frame.id);
        expect(node.globalMemoryCount).toBe(2);
        expect(node.personalMemoryCount).toBe(1);
        expect(node.estimatedTokens).toBe(1200);
        expect(node.perEntryTokens).toEqual([
            { label: "[AgentMux System] App API", source: "global", tokens: 850, sizeBytes: 3400 },
            { label: "[Workspace] Rules", source: "global", tokens: 250, sizeBytes: 1000 },
            { label: "notes.md", source: "personal", tokens: 100, sizeBytes: 400 },
        ]);
        expect(node.totalSizeBytes).toEqual({ global: 4400, personal: 400 });
        expect(node.sizeBand).toBe("low");
        expect(node.at).toBe(Date.parse(frame.timestamp));
    });

    it("tolerates a frame with missing or malformed fields", () => {
        const node = buildMemoryInjectedNode({ type: "system", subtype: "agentmux_memory_injected", entries: [null, 7] }, { now: 42 })!;
        expect(node.perEntryTokens).toEqual([]);
        expect(node.at).toBe(42);
        expect(node.id).toBe("memory-injected-notime");
    });
});

describe("history replay of the memory-injected frame", () => {
    it("renders the notice once, keyed by the frame's own id", () => {
        const line = JSON.stringify(frame);
        const { nodes } = parseHistoryLines([line, line], "claude");
        const notices = nodes.filter((n) => n.type === "memory_reinjection");
        expect(notices).toHaveLength(1);
        expect(notices[0].id).toBe(frame.id);
    });
});
