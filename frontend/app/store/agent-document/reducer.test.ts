// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { DocumentNode, ShellNode, ToolLogChunk, ToolNode } from "../../view/agent/types";
import { update } from "./reducer";
import { AgentDocumentState, initialState, TRUNCATE_GRACE_MS } from "./types";

const md = (id: string, content = id): DocumentNode => ({
    type: "markdown",
    id,
    content,
    timestamp: 0,
});

const tool = (id: string, overrides: Partial<ToolNode> = {}): ToolNode => ({
    type: "tool",
    id,
    tool: "Bash",
    params: { command: "echo hi" },
    status: "running",
    collapsed: false,
    summary: `🔧 Bash echo hi`,
    ...overrides,
});

const chunk = (
    content: string,
    overrides: Partial<ToolLogChunk> = {},
): ToolLogChunk => ({
    kind: "stdout",
    content,
    timestamp: 1000,
    ...overrides,
});

const shell = (id: string, overrides: Partial<ShellNode> = {}): ShellNode => ({
    type: "shell",
    id,
    cmd: "echo hi",
    title: "echo hi",
    status: "running",
    spawnedAt: 0,
    log: { chunks: [], open: true },
    ...overrides,
});

/**
 * Build a state with `nodes` AND matching `nodeIdSet` / `nodeIndexById`.
 * Required since issue #728 gap 4 made `nodeIdSet` part of
 * `AgentDocumentState` (bare `{ ...initialState(), nodes: [...] }` would
 * leave the index empty and break dedup invariants), and task #39 added
 * `nodeIndexById` as the O(1) position index `ToolChunkAppend` /
 * `ShellChunkAppend` / `ShellStatusUpdate` / `StreamFlush` rely on instead
 * of scanning `nodes` — same requirement, same reason.
 */
const seed = (nodes: DocumentNode[]): AgentDocumentState => ({
    ...initialState(),
    nodes,
    nodeIdSet: new Set(nodes.map((n) => n.id)),
    nodeIndexById: new Map(nodes.map((n, i) => [n.id, i])),
});

describe("agent document reducer", () => {
    describe("HistoryLoaded", () => {
        it("prepends nodes onto an empty document", () => {
            const r = update(initialState(), { type: "HistoryLoaded", nodes: [md("h1"), md("h2")] });
            expect(r.state.nodes.map((n) => n.id)).toEqual(["h1", "h2"]);
            expect(r.events).toEqual([
                { type: "history-loaded", addedCount: 2, duplicatesDropped: 0 },
            ]);
        });

        it("dedups against existing IDs", () => {
            const start = seed([md("a"), md("b")]);
            const r = update(start, { type: "HistoryLoaded", nodes: [md("a"), md("h1")] });
            expect(r.state.nodes.map((n) => n.id)).toEqual(["h1", "a", "b"]);
            expect(r.events[0]).toMatchObject({ addedCount: 1, duplicatesDropped: 1 });
        });

        it("is a no-op when all incoming nodes are duplicates", () => {
            const start = seed([md("a")]);
            const r = update(start, { type: "HistoryLoaded", nodes: [md("a")] });
            expect(r.state).toBe(start); // referentially unchanged
        });
    });

    describe("HistoryRestored (snapshot)", () => {
        it("prepends nodes onto an empty document and jumps to active", () => {
            const r = update(initialState(), {
                type: "HistoryRestored",
                fromSnapshot: true,
                nodes: [md("s1"), md("s2"), md("s3")],
            });
            expect(r.state.nodes.map((n) => n.id)).toEqual(["s1", "s2", "s3"]);
            expect(r.state.sessionPhase).toBe("active");
            expect(r.state.nodeIdSet).toEqual(new Set(["s1", "s2", "s3"]));
            expect(r.events).toEqual([{ type: "history-restored", restoredCount: 3, fromSnapshot: true }]);
        });

        it("preserves live nodes that arrived during the snapshot read window", () => {
            // Race fix (codex P1 round 4): useAgentStream may dispatch
            // StreamFlush before BlockfileReadStateCommand resolves. The
            // restored snapshot must NOT wipe those live arrivals — it
            // prepends as "older" history with dedup, like HistoryLoaded.
            const start = seed([md("live1"), md("live2")]);
            const r = update(start, {
                type: "HistoryRestored",
                fromSnapshot: true,
                nodes: [md("snap1"), md("snap2")],
            });
            expect(r.state.nodes.map((n) => n.id)).toEqual(["snap1", "snap2", "live1", "live2"]);
            expect(r.state.nodeIdSet).toEqual(new Set(["snap1", "snap2", "live1", "live2"]));
            expect(r.state.sessionPhase).toBe("active");
        });

        it("dedups snapshot nodes against existing live nodes (live version wins)", () => {
            const start = seed([md("shared", "live-content"), md("live-only")]);
            const r = update(start, {
                type: "HistoryRestored",
                fromSnapshot: true,
                nodes: [md("shared", "snap-content"), md("snap-only")],
            });
            // "shared" stays as the live version (existing node unchanged);
            // snap-only prepends; live-only stays.
            expect(r.state.nodes.map((n) => n.id)).toEqual(["snap-only", "shared", "live-only"]);
            const sharedNode = r.state.nodes.find((n) => n.id === "shared") as any;
            expect(sharedNode.content).toBe("live-content");
            // Audit event counts only the freshly-prepended snapshot nodes.
            expect(r.events).toEqual([{ type: "history-restored", restoredCount: 1, fromSnapshot: true }]);
        });

        it("empty restore is allowed and still flips sessionPhase to active", () => {
            const start = seed([md("live")]);
            const r = update(start, { type: "HistoryRestored", fromSnapshot: true, nodes: [] });
            expect(r.state.nodes.map((n) => n.id)).toEqual(["live"]);
            expect(r.state.sessionPhase).toBe("active");
        });

        it("subsequent StreamFlush appends on top of restored nodes", () => {
            const s0 = update(initialState(), {
                type: "HistoryRestored",
                fromSnapshot: true,
                nodes: [md("r1"), md("r2")],
            }).state;
            const s1 = update(s0, {
                type: "StreamFlush",
                newNodes: [md("live")],
                updatedNodes: [],
            }).state;
            expect(s1.nodes.map((n) => n.id)).toEqual(["r1", "r2", "live"]);
        });
    });

    describe("StreamFlush", () => {
        it("appends new nodes", () => {
            const r = update(initialState(), {
                type: "StreamFlush",
                newNodes: [md("s1"), md("s2")],
                updatedNodes: [],
            });
            expect(r.state.nodes.map((n) => n.id)).toEqual(["s1", "s2"]);
            expect(r.events[0]).toMatchObject({ appendedNew: 2, collidedAndUpdated: 0 });
        });

        it("history then stream produces history-then-stream order", () => {
            const s0 = update(initialState(), {
                type: "HistoryLoaded",
                nodes: [md("h1"), md("h2")],
            }).state;
            const s1 = update(s0, {
                type: "StreamFlush",
                newNodes: [md("s1")],
                updatedNodes: [],
            }).state;
            expect(s1.nodes.map((n) => n.id)).toEqual(["h1", "h2", "s1"]);
        });

        it("routes new nodes whose ID already exists into in-place update", () => {
            const s0 = update(initialState(), {
                type: "StreamFlush",
                newNodes: [md("a", "v1")],
                updatedNodes: [],
            }).state;
            const r = update(s0, {
                type: "StreamFlush",
                newNodes: [md("a", "v2")],
                updatedNodes: [],
            });
            expect(r.state.nodes).toHaveLength(1);
            expect((r.state.nodes[0] as any).content).toBe("v2");
            expect(r.events[0]).toMatchObject({ appendedNew: 0, collidedAndUpdated: 1 });
        });

        it("merges markdown updates into existing markdown content", () => {
            const s0 = update(initialState(), {
                type: "StreamFlush",
                newNodes: [md("a", "hello")],
                updatedNodes: [],
            }).state;
            const r = update(s0, {
                type: "StreamFlush",
                newNodes: [],
                updatedNodes: [md("a", "hello world")],
            });
            expect((r.state.nodes[0] as any).content).toBe("hello world");
            expect(r.events[0]).toMatchObject({ updateApplied: 1, updateDropped: 0 });
        });

        it("drops updates targeting unknown IDs", () => {
            const start = initialState();
            const r = update(start, {
                type: "StreamFlush",
                newNodes: [],
                updatedNodes: [md("ghost")],
            });
            // Reducer must return the SAME state reference when nothing changed.
            expect(r.state).toBe(start);
            expect(r.events).toEqual([]);
        });

        it("is a no-op when both lists are empty", () => {
            const start = initialState();
            const r = update(start, {
                type: "StreamFlush",
                newNodes: [],
                updatedNodes: [],
            });
            expect(r.state).toBe(start);
            expect(r.events).toEqual([]);
        });

        it("applies a same-batch update on top of a same-batch new node (text-delta race)", () => {
            // Regression: Mazs agent showed "Y" instead of "Yep, still here…"
            // because the parser pre-merged the second text_delta and the
            // reducer processed updatedNodes before newNodes — so the update
            // was dropped as orphan and the un-merged first-delta node was
            // appended. See reports/agentmux/PLAN_AGENT_PANE_TEXT_TRUNCATION_
            // FIX_2026-05-28.md.
            const r = update(initialState(), {
                type: "StreamFlush",
                newNodes:     [md("n1", "Y")],
                updatedNodes: [md("n1", "Yep, still here. What do you need?")],
            });
            expect(r.state.nodes).toHaveLength(1);
            expect((r.state.nodes[0] as any).content).toBe(
                "Yep, still here. What do you need?",
            );
            expect(r.events[0]).toMatchObject({
                appendedNew: 1,
                updateApplied: 1,
                updateDropped: 0,
            });
        });

        it("same-batch new node + update preserves new-node order", () => {
            // Two new nodes plus an update to one of them, all in one batch.
            // The appended order is still newNodes order; the update lands
            // on whichever node it targets without disturbing layout.
            const r = update(initialState(), {
                type: "StreamFlush",
                newNodes:     [md("a", "first"), md("b", "second")],
                updatedNodes: [md("a", "first-updated")],
            });
            expect(r.state.nodes.map((n) => n.id)).toEqual(["a", "b"]);
            expect((r.state.nodes[0] as any).content).toBe("first-updated");
            expect((r.state.nodes[1] as any).content).toBe("second");
        });

        it("orphan update (no same-batch new node, no prior state) is still dropped", () => {
            // Locks down the existing orphan-update contract: the fix must
            // NOT materialize ghost nodes from updates that have nothing
            // to anchor to.
            const r = update(initialState(), {
                type: "StreamFlush",
                newNodes:     [md("a", "anchor")],
                updatedNodes: [md("ghost", "drifts away")],
            });
            expect(r.state.nodes.map((n) => n.id)).toEqual(["a"]);
            expect(r.events[0]).toMatchObject({
                appendedNew: 1,
                updateApplied: 0,
                updateDropped: 1,
            });
        });
    });

    describe("StreamTruncate suppression", () => {
        it("honors truncate when not yet started (loading-history phase)", () => {
            const start = seed([md("h1")]);
            const r = update(start, { type: "StreamTruncate", reason: "fileop" }, 1000);
            expect(r.state.nodes).toEqual([]);
            expect(r.events[0]).toMatchObject({ type: "truncate-applied", clearedCount: 1 });
        });

        it("honors truncate within the grace window", () => {
            const start = update(initialState(), { type: "SessionStart", at: 1000 }).state;
            const withNodes = update(start, {
                type: "StreamFlush",
                newNodes: [md("s1")],
                updatedNodes: [],
            }).state;
            const r = update(
                withNodes,
                { type: "StreamTruncate", reason: "fileop" },
                1000 + TRUNCATE_GRACE_MS - 100,
            );
            expect(r.state.nodes).toEqual([]);
            expect(r.events[0].type).toBe("truncate-applied");
        });

        it("suppresses truncate after grace window when active session has nodes", () => {
            const start = update(initialState(), { type: "SessionStart", at: 1000 }).state;
            const withNodes = update(start, {
                type: "StreamFlush",
                newNodes: [md("s1")],
                updatedNodes: [],
            }).state;
            const r = update(
                withNodes,
                { type: "StreamTruncate", reason: "fileop" },
                1000 + TRUNCATE_GRACE_MS + 1000,
            );
            // The bug fix: nodes survive a late truncate.
            expect(r.state.nodes.map((n) => n.id)).toEqual(["s1"]);
            expect(r.events[0]).toMatchObject({
                type: "truncate-suppressed",
                reason: "fileop",
                nodeCount: 1,
            });
        });

        it("does NOT suppress truncate after grace window if document is empty", () => {
            const start = update(initialState(), { type: "SessionStart", at: 1000 }).state;
            const r = update(
                start,
                { type: "StreamTruncate", reason: "fileop" },
                1000 + TRUNCATE_GRACE_MS + 1000,
            );
            // Empty doc — no harm in honoring; nothing to lose.
            expect(r.state.nodes).toEqual([]);
            expect(r.events).toEqual([]); // already empty, so no truncate-applied event
        });
    });

    describe("UserClear", () => {
        it("always wipes regardless of session phase", () => {
            const start = update(initialState(), { type: "SessionStart", at: 1000 }).state;
            const withNodes = update(start, {
                type: "StreamFlush",
                newNodes: [md("s1"), md("s2")],
                updatedNodes: [],
            }).state;
            const r = update(
                withNodes,
                { type: "UserClear" },
                1000 + TRUNCATE_GRACE_MS + 5000,
            );
            expect(r.state.nodes).toEqual([]);
            expect(r.events[0]).toMatchObject({ type: "user-cleared", clearedCount: 2 });
        });

        it("emits an event even on empty doc (audit signal)", () => {
            const r = update(initialState(), { type: "UserClear" });
            expect(r.events[0]).toMatchObject({ type: "user-cleared", clearedCount: 0 });
        });
    });

    describe("Session phase transitions", () => {
        it("starts in loading-history phase", () => {
            expect(initialState().sessionPhase).toBe("loading-history");
        });

        it("SessionStart → active", () => {
            const r = update(initialState(), { type: "SessionStart", at: 100 });
            expect(r.state.sessionPhase).toBe("active");
            expect(r.state.sessionStartedAt).toBe(100);
        });

        it("SessionEnd → ended", () => {
            const start = update(initialState(), { type: "SessionStart", at: 100 }).state;
            const r = update(start, { type: "SessionEnd", at: 200 });
            expect(r.state.sessionPhase).toBe("ended");
            expect(r.state.sessionStartedAt).toBe(100); // preserved
        });
    });

    describe("nodeIdSet invariant (gap 4)", () => {
        it("StreamFlush adds new node ids to nodeIdSet", () => {
            const r = update(initialState(), {
                type: "StreamFlush",
                newNodes: [md("a"), md("b")],
                updatedNodes: [],
            });
            expect([...r.state.nodeIdSet].sort()).toEqual(["a", "b"]);
        });

        it("HistoryLoaded adds prepended node ids to nodeIdSet", () => {
            const r = update(initialState(), {
                type: "HistoryLoaded",
                nodes: [md("h1"), md("h2")],
            });
            expect([...r.state.nodeIdSet].sort()).toEqual(["h1", "h2"]);
        });

        it("UserClear resets nodeIdSet", () => {
            const s0 = update(initialState(), {
                type: "StreamFlush",
                newNodes: [md("a"), md("b")],
                updatedNodes: [],
            }).state;
            expect(s0.nodeIdSet.size).toBe(2);
            const r = update(s0, { type: "UserClear" });
            expect(r.state.nodeIdSet.size).toBe(0);
        });

        it("StreamTruncate (when honored) resets nodeIdSet", () => {
            const s0 = update(initialState(), {
                type: "StreamFlush",
                newNodes: [md("a")],
                updatedNodes: [],
            }).state;
            // Truncate before any session-active grace would kick in →
            // honored unconditionally (sessionPhase still loading-history).
            const r = update(s0, { type: "StreamTruncate", reason: "fileop" });
            expect(r.state.nodeIdSet.size).toBe(0);
        });

        it("StreamFlush updates do not double-add to nodeIdSet on collision", () => {
            const s0 = update(initialState(), {
                type: "StreamFlush",
                newNodes: [md("a")],
                updatedNodes: [],
            }).state;
            const r = update(s0, {
                type: "StreamFlush",
                newNodes: [md("a", "v2")], // collides → in-place update
                updatedNodes: [],
            });
            expect(r.state.nodeIdSet.size).toBe(1);
        });
    });

    describe("Injectable truncate grace (gap 6)", () => {
        it("respects opts.truncateGraceMs override", () => {
            const start = update(initialState(), { type: "SessionStart", at: 0 }).state;
            const withNodes = update(start, {
                type: "StreamFlush",
                newNodes: [md("a")],
                updatedNodes: [],
            }).state;
            // With a 100ms grace override, a truncate at t=200 should
            // suppress (200 > 100).
            const r = update(
                withNodes,
                { type: "StreamTruncate", reason: "fileop" },
                200,
                { truncateGraceMs: 100 },
            );
            expect(r.events[0].type).toBe("truncate-suppressed");
            expect(r.state.nodes).toHaveLength(1);
        });

        it("0ms grace makes any active truncate suppress immediately", () => {
            const start = update(initialState(), { type: "SessionStart", at: 0 }).state;
            const withNodes = update(start, {
                type: "StreamFlush",
                newNodes: [md("a")],
                updatedNodes: [],
            }).state;
            const r = update(
                withNodes,
                { type: "StreamTruncate", reason: "fileop" },
                1,
                { truncateGraceMs: 0 },
            );
            expect(r.events[0].type).toBe("truncate-suppressed");
        });

        it("falls back to default grace when opts omitted", () => {
            const start = update(initialState(), { type: "SessionStart", at: 1000 }).state;
            const withNodes = update(start, {
                type: "StreamFlush",
                newNodes: [md("a")],
                updatedNodes: [],
            }).state;
            // No opts → uses TRUNCATE_GRACE_MS default. Within window → honored.
            const r = update(
                withNodes,
                { type: "StreamTruncate", reason: "fileop" },
                1000 + TRUNCATE_GRACE_MS - 100,
            );
            expect(r.events[0].type).toBe("truncate-applied");
        });
    });

    describe("Purity", () => {
        it("does not mutate the input state", () => {
            const start = seed([md("a")]);
            const snapshot = {
                nodes: start.nodes.slice(),
                ids: [...start.nodeIdSet],
            };
            update(start, { type: "StreamFlush", newNodes: [md("b")], updatedNodes: [] });
            expect(start.nodes).toEqual(snapshot.nodes);
            expect([...start.nodeIdSet]).toEqual(snapshot.ids);
        });

        it("returns referentially same state when no work to do", () => {
            const start = seed([md("a")]);
            const r = update(start, { type: "StreamFlush", newNodes: [], updatedNodes: [] });
            expect(r.state).toBe(start);
        });
    });

    describe("ToolChunkAppend", () => {
        const seedWithTool = (t: ToolNode, extras: DocumentNode[] = []) =>
            seed([...extras, t]);

        it("appends one chunk to a running tool's log buffer", () => {
            const start = seedWithTool(tool("t1"));
            const r = update(start, {
                type: "ToolChunkAppend",
                toolId: "t1",
                chunk: chunk("hello\n"),
            });
            const t = r.state.nodes.find((n) => n.id === "t1") as ToolNode;
            expect(t.log?.chunks).toHaveLength(1);
            expect(t.log?.chunks[0]).toEqual({
                kind: "stdout",
                content: "hello\n",
                timestamp: 1000,
            });
            expect(t.log?.open).toBe(true);
            expect(r.events[0]).toEqual({
                type: "tool-chunk-appended",
                toolId: "t1",
                chunkCount: 1,
            });
        });

        it("preserves order across many appends", () => {
            let s = seedWithTool(tool("t1"));
            const lines = ["a\n", "b\n", "c\n", "d\n", "e\n"];
            for (let i = 0; i < lines.length; i++) {
                s = update(s, {
                    type: "ToolChunkAppend",
                    toolId: "t1",
                    chunk: chunk(lines[i], { timestamp: 1000 + i }),
                }).state;
            }
            const t = s.nodes.find((n) => n.id === "t1") as ToolNode;
            expect(t.log?.chunks.map((c) => c.content)).toEqual(lines);
        });

        it("drops chunks targeting an unknown tool id", () => {
            const start = seedWithTool(tool("t1"));
            const r = update(start, {
                type: "ToolChunkAppend",
                toolId: "ghost",
                chunk: chunk("x"),
            });
            expect(r.state).toBe(start);
            expect(r.events[0]).toEqual({
                type: "tool-chunk-dropped",
                toolId: "ghost",
                reason: "unknown-tool-id",
            });
        });

        it("drops chunks targeting a non-tool node (markdown id collision)", () => {
            const start = seed([md("m1")]);
            const r = update(start, {
                type: "ToolChunkAppend",
                toolId: "m1",
                chunk: chunk("x"),
            });
            expect(r.state).toBe(start);
            expect(r.events[0]).toEqual({
                type: "tool-chunk-dropped",
                toolId: "m1",
                reason: "node-not-tool",
            });
        });

        it("dedups the immediate re-append (history replay case)", () => {
            const start = seedWithTool(tool("t1"));
            const c = chunk("once", { timestamp: 1234 });
            const after1 = update(start, { type: "ToolChunkAppend", toolId: "t1", chunk: c }).state;
            const r = update(after1, { type: "ToolChunkAppend", toolId: "t1", chunk: c });
            const t = r.state.nodes.find((n) => n.id === "t1") as ToolNode;
            expect(t.log?.chunks).toHaveLength(1);
            expect(r.events[0]).toEqual({
                type: "tool-chunk-dropped",
                toolId: "t1",
                reason: "duplicate",
            });
            // state ref is unchanged on a dedup
            expect(r.state).toBe(after1);
        });

        it("does NOT mutate the input state", () => {
            const start = seedWithTool(tool("t1"));
            const before = {
                nodes: start.nodes.slice(),
                ids: [...start.nodeIdSet],
            };
            update(start, {
                type: "ToolChunkAppend",
                toolId: "t1",
                chunk: chunk("x"),
            });
            expect(start.nodes).toEqual(before.nodes);
            expect([...start.nodeIdSet]).toEqual(before.ids);
            // Original tool node carries no log mutation.
            const t = start.nodes.find((n) => n.id === "t1") as ToolNode;
            expect(t.log).toBeUndefined();
        });

        it("interleaves stdout and stderr in arrival order", () => {
            let s = seedWithTool(tool("t1"));
            s = update(s, {
                type: "ToolChunkAppend",
                toolId: "t1",
                chunk: chunk("out1\n", { kind: "stdout", timestamp: 1 }),
            }).state;
            s = update(s, {
                type: "ToolChunkAppend",
                toolId: "t1",
                chunk: chunk("err1\n", { kind: "stderr", timestamp: 2 }),
            }).state;
            s = update(s, {
                type: "ToolChunkAppend",
                toolId: "t1",
                chunk: chunk("out2\n", { kind: "stdout", timestamp: 3 }),
            }).state;
            const t = s.nodes.find((n) => n.id === "t1") as ToolNode;
            expect(t.log?.chunks.map((c) => `${c.kind}:${c.content.trim()}`)).toEqual([
                "stdout:out1",
                "stderr:err1",
                "stdout:out2",
            ]);
        });

        it("only mutates the targeted tool — siblings stay referentially equal", () => {
            const t1 = tool("t1");
            const t2 = tool("t2");
            const start = seed([t1, t2]);
            const r = update(start, {
                type: "ToolChunkAppend",
                toolId: "t1",
                chunk: chunk("x"),
            });
            // t1 replaced, t2 untouched.
            expect(r.state.nodes[0]).not.toBe(start.nodes[0]);
            expect(r.state.nodes[1]).toBe(start.nodes[1]);
        });
    });

    describe("ForceCancelToolNode", () => {
        it("flips a running tool node to canceled, with a distinguishing summary", () => {
            const start = seed([tool("t1", { status: "running" })]);
            const r = update(start, { type: "ForceCancelToolNode", nodeId: "t1" });
            const t = r.state.nodes.find((n) => n.id === "t1") as ToolNode;
            expect(t.status).toBe("canceled");
            expect(t.summary).toBe("⏹ Canceled — cleared via muxspect");
            expect(r.events[0]).toEqual({ type: "tool-force-canceled", nodeId: "t1" });
        });

        it("closes an open log when canceling", () => {
            const start = seed([tool("t1", { status: "running", log: { chunks: [], open: true } })]);
            const r = update(start, { type: "ForceCancelToolNode", nodeId: "t1" });
            const t = r.state.nodes.find((n) => n.id === "t1") as ToolNode;
            expect(t.log?.open).toBe(false);
        });

        it("can cancel a tool node regardless of its current status (not just running)", () => {
            // muxspect dock clear is a manual override — it shouldn't refuse
            // just because the node already resolved by the time it runs.
            const start = seed([tool("t1", { status: "success" })]);
            const r = update(start, { type: "ForceCancelToolNode", nodeId: "t1" });
            const t = r.state.nodes.find((n) => n.id === "t1") as ToolNode;
            expect(t.status).toBe("canceled");
        });

        it("no-ops on an unknown node id", () => {
            const start = seed([tool("t1")]);
            const r = update(start, { type: "ForceCancelToolNode", nodeId: "ghost" });
            expect(r.state).toBe(start);
            expect(r.events[0]).toEqual({
                type: "tool-force-cancel-skipped",
                nodeId: "ghost",
                reason: "unknown-tool-id",
            });
        });

        it("no-ops on a non-tool node (markdown id collision)", () => {
            const start = seed([md("m1")]);
            const r = update(start, { type: "ForceCancelToolNode", nodeId: "m1" });
            expect(r.state).toBe(start);
            expect(r.events[0]).toEqual({
                type: "tool-force-cancel-skipped",
                nodeId: "m1",
                reason: "node-not-tool",
            });
        });

        it("only mutates the targeted tool — siblings stay referentially equal", () => {
            const t1 = tool("t1");
            const t2 = tool("t2");
            const start = seed([t1, t2]);
            const r = update(start, { type: "ForceCancelToolNode", nodeId: "t1" });
            expect(r.state.nodes[0]).not.toBe(start.nodes[0]);
            expect(r.state.nodes[1]).toBe(start.nodes[1]);
        });

        it("is idempotent — canceling an already-canceled node is a no-op event-wise but still succeeds", () => {
            const start = seed([tool("t1", { status: "running" })]);
            const once = update(start, { type: "ForceCancelToolNode", nodeId: "t1" });
            const twice = update(once.state, { type: "ForceCancelToolNode", nodeId: "t1" });
            const t = twice.state.nodes.find((n) => n.id === "t1") as ToolNode;
            expect(t.status).toBe("canceled");
            expect(twice.events[0]).toEqual({ type: "tool-force-canceled", nodeId: "t1" });
        });
    });

    describe("StreamFlush + ToolChunkAppend interaction", () => {
        it("preserves log.chunks when tool_result replaces a running tool", () => {
            // 1. Tool starts running.
            let s = update(initialState(), {
                type: "StreamFlush",
                newNodes: [tool("t1", { status: "running" })],
                updatedNodes: [],
            }).state;
            // 2. Two chunks stream in.
            s = update(s, {
                type: "ToolChunkAppend",
                toolId: "t1",
                chunk: chunk("first\n", { timestamp: 100 }),
            }).state;
            s = update(s, {
                type: "ToolChunkAppend",
                toolId: "t1",
                chunk: chunk("second\n", { timestamp: 200 }),
            }).state;
            expect((s.nodes[0] as ToolNode).log?.chunks).toHaveLength(2);

            // 3. tool_result arrives → StreamFlush replaces the running
            //    tool node with a terminal-status one (no log on it).
            const result = update(s, {
                type: "StreamFlush",
                newNodes: [tool("t1", { status: "success", duration: 1.2 })],
                updatedNodes: [],
            });

            // The chunk buffer must survive; log.open must flip false.
            const finalTool = result.state.nodes[0] as ToolNode;
            expect(finalTool.status).toBe("success");
            expect(finalTool.duration).toBe(1.2);
            expect(finalTool.log?.chunks).toHaveLength(2);
            expect(finalTool.log?.chunks.map((c) => c.content)).toEqual(["first\n", "second\n"]);
            expect(finalTool.log?.open).toBe(false);
        });

        it("preserves log.chunks across an updatedNodes targeted update too", () => {
            let s = update(initialState(), {
                type: "StreamFlush",
                newNodes: [tool("t1", { status: "running" })],
                updatedNodes: [],
            }).state;
            s = update(s, {
                type: "ToolChunkAppend",
                toolId: "t1",
                chunk: chunk("x", { timestamp: 1 }),
            }).state;
            const result = update(s, {
                type: "StreamFlush",
                newNodes: [],
                updatedNodes: [tool("t1", { status: "failed" })],
            });
            const finalTool = result.state.nodes[0] as ToolNode;
            expect(finalTool.status).toBe("failed");
            expect(finalTool.log?.chunks).toHaveLength(1);
            expect(finalTool.log?.open).toBe(false);
        });

        it("non-tool node replacement still falls through to the unconditional path", () => {
            // Guard: mergeReplacement must not alter markdown→markdown
            // handling.
            const s0 = update(initialState(), {
                type: "StreamFlush",
                newNodes: [md("m1", "hello")],
                updatedNodes: [],
            }).state;
            // StreamFlush has a markdown-merge fast path, so this
            // exercises the "replacement is non-markdown over an
            // existing non-tool node" branch implicitly when a tool
            // result for an unknown id falls into appendedNew (it
            // doesn't, so this just verifies the markdown path still
            // works).
            const r = update(s0, {
                type: "StreamFlush",
                newNodes: [],
                updatedNodes: [md("m1", "hello world")],
            });
            expect((r.state.nodes[0] as any).content).toBe("hello world");
        });
    });

    // ── ScrubOrphanedInProgress ───────────────────────────────────
    // Spec: docs/specs/SPEC_ORPHAN_THINKING_NODES_2026_05_27.md
    //
    // The scrub runs on three triggers:
    //   - SessionEnd (clean exit) — folded into the SessionEnd handler.
    //   - HistoryRestored (snapshot load after app-kill) — folded in.
    //   - Standalone ScrubOrphanedInProgress command — explicit dispatch.
    // All three pass through the same helper.
    describe("ScrubOrphanedInProgress", () => {
        const thinkingMd = (id: string, content = id): DocumentNode => ({
            type: "markdown",
            id,
            content,
            timestamp: 0,
            metadata: { thinking: true },
        });

        it("flips thinking markdown to canceled when it's the last node", () => {
            // Spec's simple heuristic: thinking is orphaned only when
            // it's the document's tail (parser was mid-thought when
            // the session ended). Historical thinking nodes followed
            // by anything else completed normally and stay untouched.
            const s0 = seed([md("m1"), thinkingMd("t1", "mid-thought")]);
            const r = update(s0, { type: "ScrubOrphanedInProgress", at: 9999 });
            const t1 = r.state.nodes[1] as any;
            expect(t1.metadata.canceled).toBe(true);
            expect(t1.metadata.thinking).toBe(false);
            expect(t1.metadata.canceledAt).toBe(9999);
            // Original content preserved.
            expect(t1.content).toBe("mid-thought");
            // Non-thinking node untouched.
            expect((r.state.nodes[0] as any).metadata).toBeUndefined();
            expect(r.events).toEqual([
                { type: "orphans-scrubbed", markdownCanceled: 1, toolsCanceled: 0, resolvedToolNodes: [] },
            ]);
        });

        // Codex + reagent P1 on PR #1104: stream-parser never writes
        // `thinking: false` onto a completed thinking node (it only
        // clears its local pointer). So in any multi-turn session
        // — thinking → tool → text → thinking → text … — every
        // prior thinking block keeps `metadata.thinking === true`
        // forever. Scrub MUST guard against that or every reopen
        // relabels all reasoning as canceled.
        it("does NOT scrub thinking markdown followed by other nodes", () => {
            const s0 = seed([
                thinkingMd("turn1-think", "reasoned about X"),
                md("turn1-text", "answer for X"),
                thinkingMd("turn2-think", "reasoned about Y"),
                md("turn2-text", "answer for Y"),
            ]);
            const r = update(s0, { type: "ScrubOrphanedInProgress", at: 9999 });
            // No allocation, no events — both historical thinking
            // blocks were completed (text follows each one).
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([]);
        });

        // Turn-end scope (scope: "tools-only") — dispatched shortly after
        // every TurnEnd (agent-view.tsx): a foreground tool call cannot
        // outlive its turn, so a still-running tool node observed after the
        // turn ended is provably an orphan (the "running · 45m git status"
        // ghost dock row, user report 2026-08-10). Everything with a
        // session-bounded lifecycle must survive this mid-session pass.
        it("tools-only scope scrubs running tools but leaves thinking tail, live shells, and questions alone", () => {
            const s0 = seed([
                tool("ghost", { status: "running" }),
                tool("answered", { status: "awaiting_answer" }),
                shell("dev-server", { status: "running" }),
                thinkingMd("tail-think", "mid-thought"),
            ]);
            const r = update(s0, { type: "ScrubOrphanedInProgress", at: 9999, scope: "tools-only" });
            expect((r.state.nodes[0] as ToolNode).status).toBe("canceled");
            // Session-bounded lifecycles untouched:
            expect((r.state.nodes[1] as ToolNode).status).toBe("awaiting_answer");
            expect((r.state.nodes[2] as ShellNode).status).toBe("running");
            expect((r.state.nodes[3] as any).metadata.thinking).toBe(true);
            expect(r.events).toEqual([
                {
                    type: "orphans-scrubbed",
                    markdownCanceled: 0,
                    toolsCanceled: 1,
                    resolvedToolNodes: [{ id: "ghost", status: "canceled", toolName: "Bash" }],
                },
            ]);
        });

        it("tools-only scope is a no-op when no tool is running", () => {
            const s0 = seed([shell("dev-server", { status: "running" }), thinkingMd("tail-think")]);
            const r = update(s0, { type: "ScrubOrphanedInProgress", at: 9999, scope: "tools-only" });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([]);
        });

        it("scrubs tool-running anywhere in doc but leaves prior thinking alone", () => {
            // Realistic snapshot of an app-killed session: prior
            // turn's thinking → text → new turn's tool started but
            // never finished. Only the running tool is orphan.
            const s0 = seed([
                thinkingMd("prior-think", "prior reasoning"),
                md("prior-text", "prior answer"),
                tool("t1", { status: "running" }),
            ]);
            const r = update(s0, { type: "ScrubOrphanedInProgress", at: 9999 });
            expect((r.state.nodes[0] as any).metadata.canceled).toBeUndefined();
            expect((r.state.nodes[0] as any).metadata.thinking).toBe(true);
            expect((r.state.nodes[2] as ToolNode).status).toBe("canceled");
            expect(r.events).toEqual([
                {
                    type: "orphans-scrubbed",
                    markdownCanceled: 0,
                    toolsCanceled: 1,
                    resolvedToolNodes: [{ id: "t1", status: "canceled", toolName: "Bash" }],
                },
            ]);
        });

        it("flips running tools to canceled status", () => {
            const t1 = tool("t1", { status: "running" });
            const t2 = tool("t2", { status: "success" });
            const r = update(seed([t1, t2]), {
                type: "ScrubOrphanedInProgress",
                at: 1000,
            });
            expect((r.state.nodes[0] as ToolNode).status).toBe("canceled");
            // Already-completed tool stays as-is.
            expect((r.state.nodes[1] as ToolNode).status).toBe("success");
            expect(r.events).toEqual([
                {
                    type: "orphans-scrubbed",
                    markdownCanceled: 0,
                    toolsCanceled: 1,
                    resolvedToolNodes: [{ id: "t1", status: "canceled", toolName: "Bash" }],
                },
            ]);
        });

        // AskUserQuestion answered before reload (conversation continued past
        // the question) — must resolve to `success` and clear `question` so the
        // question panel does NOT re-surface. SPEC_ASK_USER_QUESTION §13/Phase 3.
        it("resolves an answered AskUserQuestion (content after) to success", () => {
            const q = tool("q1", {
                status: "awaiting_answer",
                question: { type: "ask_user_question", tool_use_id: "q1", questions: [] },
                summary: "❓ Waiting for your answer",
            });
            const after = md("m1", "the agent continued after the answer");
            const r = update(seed([q, after]), { type: "ScrubOrphanedInProgress", at: 1000 });
            const scrubbed = r.state.nodes[0] as ToolNode;
            expect(scrubbed.status).toBe("success");
            expect(scrubbed.question).toBeUndefined();
            expect(scrubbed.summary).toBe("❓ Question answered");
            expect(r.events).toEqual([
                {
                    type: "orphans-scrubbed",
                    markdownCanceled: 0,
                    toolsCanceled: 1,
                    resolvedToolNodes: [{ id: "q1", status: "success", toolName: "Bash" }],
                },
            ]);
        });

        // A TAIL AskUserQuestion is left alone — a one-shot agent's turn ends
        // (SessionEnd) with the question as the tail, and it's still
        // answerable via a follow-up resume turn. Scrubbing it would kill the
        // panel before the user could answer.
        it("leaves a tail-pending AskUserQuestion untouched", () => {
            const q = tool("q1", {
                status: "awaiting_answer",
                question: { type: "ask_user_question", tool_use_id: "q1", questions: [] },
                summary: "❓ Waiting for your answer",
            });
            const s0 = seed([md("m1"), q]);
            const r = update(s0, { type: "ScrubOrphanedInProgress", at: 1000 });
            // No allocation, no events — nothing to scrub.
            expect(r.state).toBe(s0);
            expect((r.state.nodes[1] as ToolNode).status).toBe("awaiting_answer");
            expect(r.events).toEqual([]);
        });

        // Codex + reagent P2 on #1104: scrubbing must also close
        // the streaming log. Otherwise ToolBlock's live-tail branch
        // (gated on log.open) keeps rendering a canceled tool as
        // streaming.
        it("closes log.open when canceling a streamed-into running tool", () => {
            const t1: ToolNode = {
                ...tool("t1", { status: "running" }),
                log: { chunks: [chunk("partial output")], open: true },
            };
            const r = update(seed([t1]), {
                type: "ScrubOrphanedInProgress",
                at: 1000,
            });
            const scrubbed = r.state.nodes[0] as ToolNode;
            expect(scrubbed.status).toBe("canceled");
            expect(scrubbed.log?.open).toBe(false);
            // Chunks preserved — historical output stays visible.
            expect(scrubbed.log?.chunks.length).toBe(1);
        });

        it("leaves pending_approval tools alone (decision still in flight)", () => {
            const t = tool("t1", { status: "pending_approval" });
            const r = update(seed([t]), {
                type: "ScrubOrphanedInProgress",
                at: 1000,
            });
            expect((r.state.nodes[0] as ToolNode).status).toBe("pending_approval");
            expect(r.events).toEqual([]);
        });

        it("is a no-op when nothing's in progress", () => {
            const s0 = seed([md("m1"), tool("t1", { status: "success" })]);
            const r = update(s0, { type: "ScrubOrphanedInProgress", at: 9999 });
            expect(r.state).toBe(s0); // identity — no allocation
            expect(r.events).toEqual([]);
        });

        it("is idempotent — running twice doesn't double-modify", () => {
            // Thinking must be at the tail to be scrubbed under the
            // last-node heuristic.
            const s0 = seed([tool("k1", { status: "running" }), thinkingMd("t1")]);
            const r1 = update(s0, { type: "ScrubOrphanedInProgress", at: 1000 });
            const r2 = update(r1.state, { type: "ScrubOrphanedInProgress", at: 2000 });
            // Second run is a no-op against the already-scrubbed state.
            expect(r2.state).toBe(r1.state);
            expect(r2.events).toEqual([]);
            // canceledAt was set by the FIRST run and never overwritten
            // — important: re-scrubbing must not bump the timestamp.
            const t1 = r1.state.nodes[1] as any;
            expect(t1.metadata.canceledAt).toBe(1000);
        });
    });

    describe("SessionEnd scrubs orphans", () => {
        const thinkingMd = (id: string): DocumentNode => ({
            type: "markdown",
            id,
            content: id,
            timestamp: 0,
            metadata: { thinking: true },
        });

        it("emits session-ended AND orphans-scrubbed when nodes were dirty", () => {
            // Last-node thinking + running tool earlier in the doc.
            const s0 = seed([tool("k1", { status: "running" }), thinkingMd("t1")]);
            const r = update(s0, { type: "SessionEnd", at: 5000 });
            expect(r.state.sessionPhase).toBe("ended");
            expect((r.state.nodes[0] as ToolNode).status).toBe("canceled");
            expect((r.state.nodes[1] as any).metadata.canceled).toBe(true);
            expect(r.events).toEqual([
                { type: "session-ended", at: 5000 },
                {
                    type: "orphans-scrubbed",
                    markdownCanceled: 1,
                    toolsCanceled: 1,
                    resolvedToolNodes: [{ id: "k1", status: "canceled", toolName: "Bash" }],
                },
            ]);
        });

        it("only emits session-ended when nothing was in progress", () => {
            const s0 = seed([md("m1")]);
            const r = update(s0, { type: "SessionEnd", at: 5000 });
            expect(r.events).toEqual([{ type: "session-ended", at: 5000 }]);
        });
    });

    describe("HistoryRestored scrubs orphans in the snapshot", () => {
        const thinkingMd = (id: string): DocumentNode => ({
            type: "markdown",
            id,
            content: id,
            timestamp: 0,
            metadata: { thinking: true },
        });

        it("scrubs dirty nodes that arrived via snapshot restore", () => {
            // Snapshot from a prior session was saved mid-thinking;
            // on resume, the new pane mounts and HistoryRestored
            // brings in the dirty nodes. Scrub flips them before
            // the user ever sees a misleading spinner.
            // Thinking has to be tail-of-doc to qualify as orphan
            // under the simple heuristic (anything after it means
            // the parser had already transitioned away).
            const r = update(initialState(), {
                type: "HistoryRestored",
                nodes: [tool("orphan-tool", { status: "running" }), thinkingMd("orphan-think")],
                fromSnapshot: true,
            });
            expect((r.state.nodes[0] as ToolNode).status).toBe("canceled");
            expect((r.state.nodes[1] as any).metadata.canceled).toBe(true);
            // session-phase still flips to "active" per the original
            // contract — we're restoring, not ending.
            expect(r.state.sessionPhase).toBe("active");
            expect(r.events.some((e: any) => e.type === "orphans-scrubbed")).toBe(true);
        });

        it("doesn't emit orphans-scrubbed when the snapshot is clean", () => {
            const r = update(initialState(), {
                type: "HistoryRestored",
                nodes: [md("m1"), md("m2")],
                fromSnapshot: true,
            });
            expect(r.events.some((e: any) => e.type === "orphans-scrubbed")).toBe(false);
        });

        // Codex P2 r3 on #1104: HistoryRestored represents the
        // COMPLETE pre-close history. If the snapshot's own tail is
        // mid-thought, it's the orphan candidate even when live
        // arrivals have populated state.nodes during the async read
        // window. (The pagination guard belongs to HistoryLoaded
        // only, where `fresh` is a sub-range of a larger doc.)
        it("scrubs snapshot-tail thinking even after live arrivals", () => {
            const live = md("live-text", "live content");
            const base = seed([live]);
            const r = update(base, {
                type: "HistoryRestored",
                fromSnapshot: true,
                nodes: [md("snap-1"), thinkingMd("snap-tail")],
            });
            const snapTail = r.state.nodes.find((n) => n.id === "snap-tail") as any;
            expect(snapTail.metadata.canceled).toBe(true);
            // Live node untouched.
            const liveAfter = r.state.nodes.find((n) => n.id === "live-text") as any;
            expect(liveAfter.metadata).toBeUndefined();
            expect(r.events.some((e: any) => e.type === "orphans-scrubbed")).toBe(true);
        });

        // Codex P2 on #1104: HistoryRestored arriving AFTER live
        // stream events have populated state.nodes must not flip
        // those live nodes to canceled — only the snapshot replay
        // gets sanitized.
        it("does NOT scrub live thinking nodes already in state.nodes", () => {
            // Live event landed first: an actively-streaming thinking
            // markdown is in state.nodes.
            const live = thinkingMd("live-think");
            const base = seed([live]);
            // Now the snapshot read returns — bringing only a clean
            // historical node. The live thinking should stay thinking.
            const r = update(base, {
                type: "HistoryRestored",
                nodes: [md("snap-1")],
                fromSnapshot: true,
            });
            // No orphans event — fresh was clean and live was untouched.
            expect(r.events.some((e: any) => e.type === "orphans-scrubbed")).toBe(false);
            const liveAfter = r.state.nodes.find((n) => n.id === "live-think") as any;
            expect(liveAfter.metadata.thinking).toBe(true);
            expect(liveAfter.metadata.canceled).toBeUndefined();
        });
    });

    // Codex P2 on #1104 (gap 2): HistoryLoaded is the legacy/NDJSON
    // fallback path. Contract said it scrubs; reducer wasn't doing it.
    describe("HistoryLoaded scrubs orphans in the replay", () => {
        const thinkingMd = (id: string): DocumentNode => ({
            type: "markdown",
            id,
            content: id,
            timestamp: 0,
            metadata: { thinking: true },
        });

        it("flips dirty nodes from the legacy/NDJSON replay", () => {
            const r = update(initialState(), {
                type: "HistoryLoaded",
                nodes: [tool("orphan-tool", { status: "running" }), thinkingMd("orphan-think")],
            });
            expect((r.state.nodes[0] as ToolNode).status).toBe("canceled");
            expect((r.state.nodes[1] as any).metadata.canceled).toBe(true);
            expect(r.events.some((e: any) => e.type === "orphans-scrubbed")).toBe(true);
        });

        it("doesn't emit orphans-scrubbed when the replay is clean", () => {
            const r = update(initialState(), {
                type: "HistoryLoaded",
                nodes: [md("m1"), md("m2")],
            });
            expect(r.events.some((e: any) => e.type === "orphans-scrubbed")).toBe(false);
        });

        it("leaves live thinking nodes already in state.nodes untouched", () => {
            const live = thinkingMd("live-think");
            const base = seed([live]);
            const r = update(base, {
                type: "HistoryLoaded",
                nodes: [md("hist-1")],
            });
            const liveAfter = r.state.nodes.find((n) => n.id === "live-think") as any;
            expect(liveAfter.metadata.thinking).toBe(true);
            expect(liveAfter.metadata.canceled).toBeUndefined();
        });

        // Codex P2 round 2 on #1104: useHistoryPagination.loadOlder
        // dispatches HistoryLoaded with an OLDER page while newer
        // nodes are already in state.nodes. fresh.last is the page's
        // tail but NOT the merged document's tail — a completed
        // thinking block at the end of the page must stay alive.
        it("does NOT scrub a thinking tail when older page prepends to existing state", () => {
            const newer = seed([md("text-3"), tool("tool-4", { status: "success" })]);
            const r = update(newer, {
                type: "HistoryLoaded",
                nodes: [md("text-1"), thinkingMd("think-2")],
            });
            // Merge order: [text-1, think-2, text-3, tool-4]
            // think-2 is mid-doc in the merged view; it completed.
            const think2 = r.state.nodes.find((n) => n.id === "think-2") as any;
            expect(think2.metadata.thinking).toBe(true);
            expect(think2.metadata.canceled).toBeUndefined();
            expect(r.events.some((e: any) => e.type === "orphans-scrubbed")).toBe(false);
        });

        // Even with `hasContentAfter`, a running tool in the older
        // page IS orphan (status field is positional-independent —
        // running means tool_result never arrived).
        it("still scrubs running tools in older pages", () => {
            const newer = seed([md("text-3")]);
            const r = update(newer, {
                type: "HistoryLoaded",
                nodes: [tool("old-tool", { status: "running" }), md("text-1")],
            });
            const oldTool = r.state.nodes.find((n) => n.id === "old-tool") as ToolNode;
            expect(oldTool.status).toBe("canceled");
            expect(r.events.some((e: any) => e.type === "orphans-scrubbed")).toBe(true);
        });
    });

    describe("nodeIndexById invariant (task #39)", () => {
        // The reducer keeps an id->index map in lockstep with `nodes` so
        // ToolChunkAppend/ShellChunkAppend/ShellStatusUpdate/StreamFlush can
        // resolve a target in O(1) instead of scanning the whole array on
        // every streamed chunk. These tests pin the map's correctness across
        // every command that touches it — a drift here would silently make
        // chunk-append start dropping/misrouting chunks instead of erroring.

        it("StreamFlush keeps nodeIndexById in sync with appended node positions", () => {
            const r = update(initialState(), {
                type: "StreamFlush",
                newNodes: [md("a"), md("b"), md("c")],
                updatedNodes: [],
            });
            expect(r.state.nodeIndexById.get("a")).toBe(0);
            expect(r.state.nodeIndexById.get("b")).toBe(1);
            expect(r.state.nodeIndexById.get("c")).toBe(2);
            for (const [id, idx] of r.state.nodeIndexById) {
                expect(r.state.nodes[idx]?.id).toBe(id);
            }
        });

        it("StreamFlush collision (in-place update) does not change the index", () => {
            const s0 = update(initialState(), {
                type: "StreamFlush",
                newNodes: [md("a"), md("b")],
                updatedNodes: [],
            }).state;
            const r = update(s0, {
                type: "StreamFlush",
                newNodes: [md("a", "v2")], // collides with "a" → in-place update, not append
                updatedNodes: [],
            });
            expect(r.state.nodeIndexById.get("a")).toBe(0);
            expect(r.state.nodeIndexById.get("b")).toBe(1);
            expect((r.state.nodes[0] as DocumentNode & { content: string }).content).toBe("v2");
        });

        it("HistoryLoaded shifts existing indices by the prepended page length", () => {
            const s0 = seed([md("live-1"), md("live-2")]);
            const r = update(s0, {
                type: "HistoryLoaded",
                nodes: [md("h1"), md("h2"), md("h3")],
            });
            expect(r.state.nodeIndexById.get("h1")).toBe(0);
            expect(r.state.nodeIndexById.get("h2")).toBe(1);
            expect(r.state.nodeIndexById.get("h3")).toBe(2);
            expect(r.state.nodeIndexById.get("live-1")).toBe(3);
            expect(r.state.nodeIndexById.get("live-2")).toBe(4);
            for (const [id, idx] of r.state.nodeIndexById) {
                expect(r.state.nodes[idx]?.id).toBe(id);
            }
        });

        it("HistoryRestored (snapshot) shifts existing indices the same way", () => {
            const s0 = seed([md("live-1")]);
            const r = update(s0, {
                type: "HistoryRestored",
                nodes: [md("snap-1"), md("snap-2")],
                fromSnapshot: true,
            });
            expect(r.state.nodeIndexById.get("snap-1")).toBe(0);
            expect(r.state.nodeIndexById.get("snap-2")).toBe(1);
            expect(r.state.nodeIndexById.get("live-1")).toBe(2);
        });

        it("UserClear and StreamTruncate reset nodeIndexById to empty", () => {
            const s0 = update(initialState(), {
                type: "StreamFlush",
                newNodes: [md("a")],
                updatedNodes: [],
            }).state;
            expect(update(s0, { type: "UserClear" }).state.nodeIndexById.size).toBe(0);
            expect(
                update(s0, { type: "StreamTruncate", reason: "fileop" }).state.nodeIndexById.size,
            ).toBe(0);
        });

        it("ToolChunkAppend resolves the target via the index (O(1) path) and leaves siblings' indices untouched", () => {
            const s0 = seed([md("before"), tool("t1"), md("after")]);
            const r = update(s0, {
                type: "ToolChunkAppend",
                toolId: "t1",
                chunk: chunk("hello"),
            });
            expect((r.state.nodes[1] as ToolNode).log?.chunks.map((c) => c.content)).toEqual([
                "hello",
            ]);
            // Index map unchanged — only content at index 1 was replaced.
            expect(r.state.nodeIndexById).toBe(s0.nodeIndexById);
        });

        it("ToolChunkAppend still drops a chunk whose id maps to a non-tool node", () => {
            const s0 = seed([md("m1")]);
            const r = update(s0, {
                type: "ToolChunkAppend",
                toolId: "m1",
                chunk: chunk("x"),
            });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([
                { type: "tool-chunk-dropped", toolId: "m1", reason: "node-not-tool" },
            ]);
        });

        it("ShellNodeCreate appends and records the new index", () => {
            const s0 = seed([md("a")]);
            const r = update(s0, { type: "ShellNodeCreate", node: shell("sh1") });
            expect(r.state.nodeIndexById.get("sh1")).toBe(1);
            expect(r.state.nodes[1]?.id).toBe("sh1");
        });

        it("ShellChunkAppend resolves via the index and drops for an unknown id", () => {
            const s0 = seed([shell("sh1")]);
            const r = update(s0, {
                type: "ShellChunkAppend",
                shellId: "sh1",
                chunk: chunk("out"),
            });
            expect((r.state.nodes[0] as ShellNode).log.chunks.map((c) => c.content)).toEqual([
                "out",
            ]);
            const dropped = update(s0, {
                type: "ShellChunkAppend",
                shellId: "nope",
                chunk: chunk("out"),
            });
            expect(dropped.state).toBe(s0);
        });

        it("ShellStatusUpdate resolves via the index", () => {
            const s0 = seed([shell("sh1")]);
            const r = update(s0, {
                type: "ShellStatusUpdate",
                shellId: "sh1",
                status: "exited-ok",
                exitCode: 0,
                exitedAt: 123,
            });
            expect((r.state.nodes[0] as ShellNode).status).toBe("exited-ok");
        });
    });
});
