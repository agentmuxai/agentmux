// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { DocumentNode } from "../types";
import {
    initialStickyFrontierId,
    locateIndex,
    partitionForVirtualization,
    STREAMING_BUFFER_SIZE,
    nodeBytes,
    turnScopedFrontier,
} from "./streaming-buffer";

const md = (id: string): DocumentNode => ({ type: "markdown", id, content: id, timestamp: 0 });

const range = (n: number): DocumentNode[] =>
    Array.from({ length: n }, (_, i) => md(`n${i}`));

describe("partitionForVirtualization", () => {
    it("puts everything in streamingNodes when document <= bufferSize", () => {
        const nodes = range(10);
        const p = partitionForVirtualization(nodes, 50);
        expect(p.virtualizedNodes).toHaveLength(0);
        expect(p.streamingNodes).toBe(nodes); // identity — no slicing
        expect(p.splitIndex).toBe(0);
    });

    it("splits at length - bufferSize when document > bufferSize", () => {
        const nodes = range(70); // 50 streaming, 20 virtualized
        const p = partitionForVirtualization(nodes, 50);
        expect(p.virtualizedNodes).toHaveLength(20);
        expect(p.streamingNodes).toHaveLength(50);
        expect(p.splitIndex).toBe(20);
        // Virtualized side is the head (oldest); streaming side is the tail (newest).
        expect(p.virtualizedNodes[0].id).toBe("n0");
        expect(p.virtualizedNodes[19].id).toBe("n19");
        expect(p.streamingNodes[0].id).toBe("n20");
        expect(p.streamingNodes[49].id).toBe("n69");
    });

    it("handles exactly bufferSize nodes (boundary)", () => {
        const nodes = range(50);
        const p = partitionForVirtualization(nodes, 50);
        expect(p.virtualizedNodes).toHaveLength(0);
        expect(p.streamingNodes).toBe(nodes);
        expect(p.splitIndex).toBe(0);
    });

    it("handles bufferSize+1 (smallest case where virtualization activates)", () => {
        const nodes = range(51);
        const p = partitionForVirtualization(nodes, 50);
        expect(p.virtualizedNodes).toHaveLength(1);
        expect(p.streamingNodes).toHaveLength(50);
        expect(p.virtualizedNodes[0].id).toBe("n0");
    });

    it("uses STREAMING_BUFFER_SIZE as default", () => {
        const nodes = range(STREAMING_BUFFER_SIZE + 10);
        const p = partitionForVirtualization(nodes);
        expect(p.virtualizedNodes).toHaveLength(10);
        expect(p.streamingNodes).toHaveLength(STREAMING_BUFFER_SIZE);
    });

    it("handles empty document", () => {
        const p = partitionForVirtualization([]);
        expect(p.virtualizedNodes).toHaveLength(0);
        expect(p.streamingNodes).toHaveLength(0);
        expect(p.splitIndex).toBe(0);
    });
});

describe("partitionForVirtualization — sticky frontier", () => {
    it("splits at the frontier id when supplied (independent of count)", () => {
        // 70 nodes; anchor at n25 (which would NOT be the count-based
        // split point of n20). Streaming buffer contains n25..n69 (45
        // nodes), virtualized contains n0..n24 (25 nodes).
        const nodes = range(70);
        const p = partitionForVirtualization(nodes, 50, "n25");
        expect(p.splitIndex).toBe(25);
        expect(p.virtualizedNodes).toHaveLength(25);
        expect(p.streamingNodes).toHaveLength(45);
        expect(p.streamingNodes[0].id).toBe("n25");
    });

    it("never migrates a node across the split on a simple append", () => {
        // Set the frontier with the document at 70 nodes, then append
        // 50 more. Streaming grows from 45 → 95; virtualized stays at
        // n0..n24. Crucially: every id that was in `streamingNodes`
        // before is STILL in `streamingNodes` after.
        const before = range(70);
        const frontier = initialStickyFrontierId(before, 50);
        expect(frontier).toBe("n20"); // first call sets it to length-buffer

        const after = [...before, ...Array.from({ length: 50 }, (_, i) => md(`x${i}`))];
        const p = partitionForVirtualization(after, 50, frontier);

        const streamIdsBefore = new Set(
            partitionForVirtualization(before, 50, frontier).streamingNodes.map(
                (n) => n.id,
            ),
        );
        for (const id of streamIdsBefore) {
            expect(p.streamingNodes.map((n) => n.id)).toContain(id);
        }
        // And the new tail nodes also landed in streaming.
        expect(p.streamingNodes.map((n) => n.id)).toContain("x49");
    });

    it("returns splitIndex=-1 when the frontier id is stale", () => {
        // Document truncated — the anchor node no longer exists.
        const nodes = range(70);
        const p = partitionForVirtualization(nodes, 50, "deleted-id");
        expect(p.splitIndex).toBe(-1);
    });

    it("ignores the frontier when the document is within the buffer", () => {
        // Document hasn't crossed the threshold yet; everything streams.
        // No sticky behavior should kick in.
        const nodes = range(20);
        const p = partitionForVirtualization(nodes, 50, "n5");
        expect(p.splitIndex).toBe(0);
        expect(p.virtualizedNodes).toHaveLength(0);
        expect(p.streamingNodes).toBe(nodes);
    });

    it("falls back to count-based split when no frontier supplied", () => {
        const nodes = range(70);
        const p = partitionForVirtualization(nodes, 50, null);
        expect(p.splitIndex).toBe(20);
    });
});

describe("initialStickyFrontierId", () => {
    it("returns null when the document fits in the buffer", () => {
        expect(initialStickyFrontierId(range(20), 50)).toBeNull();
        expect(initialStickyFrontierId(range(50), 50)).toBeNull();
    });

    it("returns the id at length-buffer when the document exceeds the buffer", () => {
        // 70 nodes, buffer 50 → first streaming node is at index 20
        // → frontier should be n20.
        expect(initialStickyFrontierId(range(70), 50)).toBe("n20");
    });

    it("uses STREAMING_BUFFER_SIZE as default", () => {
        // length = bufferSize + 1 → frontier at index 1 → n1
        const nodes = range(STREAMING_BUFFER_SIZE + 1);
        expect(initialStickyFrontierId(nodes)).toBe("n1");
    });
});

describe("locateIndex", () => {
    it("locates indices in the virtualized region", () => {
        const p = partitionForVirtualization(range(70), 50);
        // splitIndex=20; index 0 is virtualized, relativeIndex=0.
        expect(locateIndex(0, p)).toEqual({ side: "virtualized", relativeIndex: 0 });
        expect(locateIndex(19, p)).toEqual({ side: "virtualized", relativeIndex: 19 });
    });

    it("locates indices in the streaming region", () => {
        const p = partitionForVirtualization(range(70), 50);
        // splitIndex=20; index 20 is streaming side, relativeIndex 0.
        expect(locateIndex(20, p)).toEqual({ side: "streaming", relativeIndex: 0 });
        expect(locateIndex(69, p)).toEqual({ side: "streaming", relativeIndex: 49 });
    });

    it("returns null for out-of-range indices", () => {
        const p = partitionForVirtualization(range(70), 50);
        expect(locateIndex(-1, p)).toBeNull();
        expect(locateIndex(70, p)).toBeNull();
        expect(locateIndex(999, p)).toBeNull();
    });

    it("handles empty partition", () => {
        const p = partitionForVirtualization([]);
        expect(locateIndex(0, p)).toBeNull();
    });

    it("locates the only index in a single-node streaming partition", () => {
        const p = partitionForVirtualization(range(1));
        expect(locateIndex(0, p)).toEqual({ side: "streaming", relativeIndex: 0 });
    });
});

// ── Phase 3: the tail holds only the turn in flight ─────────────────────────
// SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md §6.2.

const user = (id: string): DocumentNode => ({ type: "user_message", id, message: id, timestamp: 0 });
const tool = (id: string, status: "running" | "pending_approval" | "awaiting_answer" | "success" | "failed" = "success"): DocumentNode =>
    ({ type: "tool", id, tool: "Bash", params: {}, status, collapsed: true, summary: id }) as DocumentNode;
const big = (id: string, bytes: number): DocumentNode => ({ type: "markdown", id, content: "x".repeat(bytes), timestamp: 0 });
const ids = (nodes: readonly DocumentNode[], from: number) => nodes.slice(from).map((n) => n.id);

describe("turnScopedFrontier", () => {
    it("starts the tail at the last user message", () => {
        const nodes = [user("u1"), md("a1"), tool("t1"), user("u2"), md("a2"), tool("t2")];
        expect(ids(nodes, turnScopedFrontier(nodes))).toEqual(["u2", "a2", "t2"]);
    });

    it("keeps an earlier node that is still in progress, and everything after it", () => {
        for (const status of ["running", "pending_approval", "awaiting_answer"] as const) {
            const nodes = [user("u1"), tool("t1", status), md("a1"), user("u2"), md("a2")];
            expect(ids(nodes, turnScopedFrontier(nodes)), status).toEqual(["t1", "a1", "u2", "a2"]);
        }
    });

    it("a running shell counts as in progress; a finished one does not", () => {
        const shell = (id: string, status: "running" | "exited-ok"): DocumentNode =>
            ({ type: "shell", id, cmd: "x", title: "x", status, spawnedAt: 0, log: { open: status === "running", chunks: [] } }) as DocumentNode;
        const running = [user("u1"), shell("s1", "running"), user("u2")];
        expect(ids(running, turnScopedFrontier(running))).toEqual(["s1", "u2"]);
        const done = [user("u1"), shell("s1", "exited-ok"), user("u2")];
        expect(ids(done, turnScopedFrontier(done))).toEqual(["u2"]);
    });

    it("with no user message at all, the whole document is the turn (bounded by the ceiling)", () => {
        const nodes = range(5);
        expect(turnScopedFrontier(nodes)).toBe(0);
    });

    it("a turn longer than the node ceiling keeps only its last maxNodes nodes", () => {
        const nodes = [user("u1"), ...range(60)];
        const at = turnScopedFrontier(nodes, { maxNodes: 40, maxBytes: Infinity });
        expect(nodes.length - at).toBe(40);
        expect(nodes[at].id).toBe("n20");
    });

    it("a turn over the byte ceiling sheds its oldest finished nodes until it fits", () => {
        const nodes = [user("u1"), big("b1", 300), big("b2", 300), big("b3", 300), md("last")];
        // Exactly room for the last three.
        const budget = nodeBytes(nodes[2]) + nodeBytes(nodes[3]) + nodeBytes(nodes[4]);
        const at = turnScopedFrontier(nodes, { maxNodes: Infinity, maxBytes: budget });
        expect(ids(nodes, at)).toEqual(["b2", "b3", "last"]);
        // One byte less and b2 goes too.
        expect(ids(nodes, turnScopedFrontier(nodes, { maxNodes: Infinity, maxBytes: budget - 1 }))).toEqual(["b3", "last"]);
    });

    it("the ceiling never moves past a node that is still in progress", () => {
        const nodes = [user("u1"), tool("t1", "running"), ...range(60)];
        const at = turnScopedFrontier(nodes, { maxNodes: 10, maxBytes: Infinity });
        expect(nodes[at].id).toBe("t1");
    });

    it("always keeps at least the last node", () => {
        const nodes = [big("huge", 10_000)];
        expect(turnScopedFrontier(nodes, { maxNodes: 40, maxBytes: 100 })).toBe(0);
        expect(turnScopedFrontier([], { maxNodes: 40, maxBytes: 100 })).toBe(0);
    });
});
