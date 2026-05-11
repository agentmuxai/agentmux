// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { DocumentNode, ToolLogChunk, ToolNode } from "../../view/agent/types";
import { update } from "./reducer";
import { initialState, TRUNCATE_GRACE_MS } from "./types";

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

/**
 * Build a state with `nodes` AND a matching `nodeIdSet`. Required since
 * issue #728 gap 4 made `nodeIdSet` part of `AgentDocumentState` — bare
 * `{ ...initialState(), nodes: [...] }` would leave the index empty
 * and break dedup invariants.
 */
const seed = (nodes: DocumentNode[]) => ({
    ...initialState(),
    nodes,
    nodeIdSet: new Set(nodes.map((n) => n.id)),
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
});
