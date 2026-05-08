// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { DocumentNode } from "../../view/agent/types";
import { update } from "./reducer";
import { initialState, TRUNCATE_GRACE_MS } from "./types";

const md = (id: string, content = id): DocumentNode => ({
    type: "markdown",
    id,
    content,
    timestamp: 0,
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
});
