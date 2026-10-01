// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { buildMemoryInjectedNode, isMemoryInjectedFrame } from "./memory-injected";
import { parseHistoryLines } from "./parseHistoryLines";

// The frame `server/memory_delivery_handlers.rs` `notice_frame` writes (CD2a).
const frame = {
    type: "system",
    subtype: "agentmux_memory_injected",
    id: "memory-injected-s1-compact-1790000000000",
    reason: "compact",
    session_id: "s1",
    parts: 2,
    entries: [
        {
            label: "[AgentMux System] App API",
            source: "global",
            size_bytes: 3400,
            tokens: 850,
            kind: "global_memory",
            name: "App API",
            tier: "system",
            bundle_id: "b-1",
            delivered: "full",
        },
        {
            label: "[Workspace] Rules",
            source: "global",
            size_bytes: 600,
            tokens: 150,
            kind: "global_memory",
            name: "Rules",
            tier: "workspace",
            bundle_id: "b-2",
            delivered: "partial",
        },
        {
            label: "notes.md",
            source: "personal",
            size_bytes: 0,
            tokens: 0,
            kind: "personal_memory",
            name: "notes.md",
            path: "/mem/notes.md",
            delivered: "omitted",
        },
        { label: "Running summary", source: "summary", size_bytes: 0, tokens: 0, kind: "running_summary", name: "Running summary (AgentMux)", delivered: "omitted" },
    ],
    summary_bytes: 900,
    timestamp: "2026-09-27T08:00:00+00:00",
};

// A frame written before CD2a: label and source only.
const oldFrame = {
    type: "system",
    subtype: "agentmux_memory_injected",
    id: "memory-injected-s0-startup-1789000000000",
    reason: "startup",
    entries: [
        { label: "[AgentMux System] App API", source: "global", size_bytes: 3400, tokens: 850 },
        { label: "[Workspace] Rules", source: "global", size_bytes: 1000, tokens: 250 },
        { label: "notes.md", source: "personal", size_bytes: 400, tokens: 100 },
    ],
    timestamp: "2026-09-27T07:00:00+00:00",
};

describe("buildMemoryInjectedNode — startup files (LC2)", () => {
    const startupFrame = {
        type: "system",
        subtype: "agentmux_memory_injected",
        id: "memory-injected-s1-startup-1",
        reason: "startup",
        entries: [
            {
                label: "~/.agentmux/agents/CLAUDE.md",
                source: "startup",
                kind: "startup_file",
                name: "~/.agentmux/agents/CLAUDE.md",
                path: "C:/Users/u/.agentmux/agents/CLAUDE.md",
                size_bytes: 24000,
                tokens: 6000,
                delivered: "full",
                role: "instructions",
                owner: "external",
            },
            {
                label: "~/.agentmux/agents/a/.claude/AGENTMUX_MEMORY.md",
                source: "startup",
                kind: "startup_file",
                name: "~/.agentmux/agents/a/.claude/AGENTMUX_MEMORY.md",
                size_bytes: 10000,
                tokens: 2500,
                role: "instructions_import",
                owner: "agentmux",
                contains: ["global_memory", "skills_index"],
            },
            { label: "MCP servers: agentmux", source: "startup", kind: "startup_file", name: "MCP servers: agentmux", size_bytes: 0, tokens: 0, role: "mcp_servers", owner: "agentmux", count: 1 },
            { label: "[AgentMux System] App API", source: "global", kind: "global_memory", name: "App API", tier: "system", size_bytes: 3400, tokens: 850 },
        ],
        timestamp: "2026-10-01T07:00:00+00:00",
    };

    it("reads each startup file's role, owner, count and what it carries", () => {
        const items = buildMemoryInjectedNode(startupFrame, { now: 0 })!.items;
        expect(items.map((i) => i.kind)).toEqual(["startup_file", "startup_file", "startup_file", "global_memory"]);
        expect(items[0]).toMatchObject({ role: "instructions", owner: "external", path: "C:/Users/u/.agentmux/agents/CLAUDE.md" });
        expect(items[1].contains).toEqual(["global_memory", "skills_index"]);
        expect(items[2]).toMatchObject({ role: "mcp_servers", count: 1 });
        expect(items[3].owner).toBeUndefined();
    });

    it("ignores an owner it doesn't know", () => {
        const odd = { ...startupFrame, entries: [{ ...startupFrame.entries[0], owner: "someone" }] };
        expect(buildMemoryInjectedNode(odd, { now: 0 })!.items[0].owner).toBeUndefined();
    });
});

describe("buildMemoryInjectedNode", () => {
    it("recognises only the memory-injected frame", () => {
        expect(isMemoryInjectedFrame(frame)).toBe(true);
        expect(isMemoryInjectedFrame({ type: "system", subtype: "agentmux_session_outcome" })).toBe(false);
        expect(buildMemoryInjectedNode({ type: "assistant" }, { now: 0 })).toBeNull();
    });

    it("builds a context delivery with one item per entry, and its detail", () => {
        const node = buildMemoryInjectedNode(frame, { contextWindow: 200_000, now: 0 })!;
        expect(node.type).toBe("context_delivery");
        expect(node.id).toBe(frame.id);
        expect(node.reason).toBe("compaction");
        expect(node.timestamp).toBe(Date.parse(frame.timestamp));
        expect(node.items).toEqual([
            { kind: "global_memory", name: "App API", tier: "system", bundleId: "b-1", sizeBytes: 3400, tokens: 850 },
            { kind: "global_memory", name: "Rules", tier: "workspace", bundleId: "b-2", delivered: "partial", sizeBytes: 600, tokens: 150 },
            { kind: "personal_memory", name: "notes.md", path: "/mem/notes.md", delivered: "omitted", sizeBytes: 0, tokens: 0 },
            { kind: "running_summary", name: "Running summary (AgentMux)", delivered: "omitted", sizeBytes: 0, tokens: 0 },
        ]);
        expect(node.sizeBand).toBe("low");
    });

    it("recovers names and tiers from an old frame's labels", () => {
        const node = buildMemoryInjectedNode(oldFrame, { now: 0 })!;
        expect(node.reason).toBe("startup");
        expect(node.items.map((i) => [i.kind, i.name, i.tier])).toEqual([
            ["global_memory", "App API", "system"],
            ["global_memory", "Rules", "workspace"],
            ["personal_memory", "notes.md", undefined],
        ]);
        expect(node.items.every((i) => i.delivered === undefined)).toBe(true);
    });

    it("bands Personal Memory by its size against the context window", () => {
        const big = { ...oldFrame, entries: [{ label: "huge.md", source: "personal", size_bytes: 400_000, tokens: 100_000 }] };
        expect(buildMemoryInjectedNode(big, { contextWindow: 200_000, now: 0 })!.sizeBand).not.toBe("low");
    });

    it("bands on the whole of Personal Memory, even when the delivery cut it", () => {
        // An omitted file reports 0 delivered tokens; its source size still counts.
        const cut = {
            ...frame,
            entries: [
                { label: "huge.md", source: "personal", kind: "personal_memory", name: "huge.md", delivered: "omitted", size_bytes: 0, tokens: 0, source_size_bytes: 400_000, source_tokens: 100_000 },
            ],
        };
        const node = buildMemoryInjectedNode(cut, { contextWindow: 200_000, now: 0 })!;
        expect(node.items[0].tokens).toBe(0);
        expect(node.items[0].sourceTokens).toBe(100_000);
        expect(node.sizeBand).not.toBe("low");
    });

    it("tolerates a frame with missing or malformed fields", () => {
        const node = buildMemoryInjectedNode({ type: "system", subtype: "agentmux_memory_injected", entries: [null, 7] }, { now: 42 })!;
        expect(node.items).toEqual([]);
        expect(node.timestamp).toBe(42);
        expect(node.id).toBe("memory-injected-notime");
        expect(node.reason).toBe("startup");
    });
});

describe("history replay of the memory-injected frame", () => {
    it("renders the card once, keyed by the frame's own id", () => {
        const line = JSON.stringify(frame);
        const { nodes } = parseHistoryLines([line, line], "claude");
        const cards = nodes.filter((n) => n.type === "context_delivery");
        expect(cards).toHaveLength(1);
        expect(cards[0].id).toBe(frame.id);
    });
});
