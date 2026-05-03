// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { update } from "./reducer";
import { initialState } from "./types";

const mk = () => initialState("test-agent");

describe("agent-pane-state reducer", () => {
    describe("Stream lifecycle", () => {
        it("StreamSubscribe sets active + lastEventTime", () => {
            const r = update(mk(), { type: "StreamSubscribe", at: 100 });
            expect(r.state.streaming.active).toBe(true);
            expect(r.state.streaming.lastEventTime).toBe(100);
            expect(r.events[0]).toMatchObject({ type: "stream-subscribed", at: 100 });
        });

        it("StreamUnsubscribe clears active and force-clears turnActive", () => {
            const s0 = update(mk(), { type: "StreamSubscribe", at: 100 }).state;
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            expect(s1.turnActive).toBe(true);
            const r = update(s1, { type: "StreamUnsubscribe", at: 200 });
            expect(r.state.streaming.active).toBe(false);
            expect(r.state.turnActive).toBe(false);
        });

        it("StreamFlushObserved bumps bufferSize when active", () => {
            const s0 = update(mk(), { type: "StreamSubscribe", at: 100 }).state;
            const r = update(s0, { type: "StreamFlushObserved", addedCount: 3, at: 110 });
            expect(r.state.streaming.bufferSize).toBe(3);
            expect(r.state.streaming.lastEventTime).toBe(110);
        });

        it("StreamFlushObserved is no-op when stream inactive", () => {
            const start = mk();
            const r = update(start, { type: "StreamFlushObserved", addedCount: 3, at: 110 });
            // Reducer must return SAME reference when no work was done.
            expect(r.state).toBe(start);
            expect(r.events).toEqual([]);
        });
    });

    describe("Turn lifecycle invariants", () => {
        it("TurnStart while stream inactive is suppressed", () => {
            const start = mk();
            const r = update(start, { type: "TurnStart", at: 100 });
            expect(r.state.turnActive).toBe(false);
            expect(r.state).toBe(start);
            expect(r.events[0]).toMatchObject({ type: "turn-start-suppressed" });
        });

        it("TurnStart while active sets turnActive + clears stale stats", () => {
            const s0 = update(mk(), { type: "StreamSubscribe", at: 100 }).state;
            const sWithStats = { ...s0, sessionStats: { input_tokens: 50, output_tokens: 100 } };
            const r = update(sWithStats, { type: "TurnStart", at: 110 });
            expect(r.state.turnActive).toBe(true);
            expect(r.state.sessionStats).toBe(null);
        });

        it("TurnEnd clears tool/tokens/turnActive AND stopping in one shot", () => {
            const s0 = update(mk(), { type: "StreamSubscribe", at: 100 }).state;
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, { type: "ToolStart", name: "Read" }).state;
            const s3 = update(s2, { type: "TokensIn", input: 50 }).state;
            const s4 = update(s3, { type: "TokensOut", output: 200 }).state;
            const s5 = update(s4, { type: "RequestStop", at: 120 }).state;
            const r = update(s5, {
                type: "TurnEnd",
                stats: null,
            });
            expect(r.state.turnActive).toBe(false);
            expect(r.state.currentTool).toBe(null);
            expect(r.state.turnTokens).toBe(null);
            expect(r.state.stopping).toBe(false);
            // Stats merged from live tokens (mergeStats fallback path).
            expect(r.state.sessionStats).toEqual({ input_tokens: 50, output_tokens: 200 });
            expect(r.events[0]).toMatchObject({
                type: "turn-ended",
                statsMerged: true,
                stoppingCleared: true,
            });
        });

        it("TurnEnd with explicit stats merges live token totals", () => {
            const s0 = update(mk(), { type: "StreamSubscribe", at: 100 }).state;
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, { type: "TokensIn", input: 80 }).state;
            const r = update(s2, {
                type: "TurnEnd",
                stats: { input_tokens: 0, output_tokens: 0, total_cost_usd: 0.05 } as any,
            });
            expect(r.state.sessionStats).toMatchObject({
                input_tokens: 80,
                output_tokens: 0,
                total_cost_usd: 0.05,
            });
        });

        it("TurnReset clears turn-scoped state but keeps streaming + pending", () => {
            const s0 = update(mk(), { type: "StreamSubscribe", at: 100 }).state;
            const s1 = update(s0, {
                type: "PendingMessageQueued",
                id: "p1",
                text: "hello",
                at: 105,
            }).state;
            const s2 = update(s1, { type: "TurnStart", at: 110 }).state;
            const s3 = update(s2, { type: "ToolStart", name: "Edit" }).state;
            const r = update(s3, { type: "TurnReset" });
            expect(r.state.streaming.active).toBe(true); // preserved
            expect(r.state.pending).toHaveLength(1); // preserved
            expect(r.state.currentTool).toBe(null);
            expect(r.state.turnActive).toBe(false);
        });
    });

    describe("Tool", () => {
        it("ToolStart sets currentTool", () => {
            const r = update(mk(), { type: "ToolStart", name: "Bash" });
            expect(r.state.currentTool).toBe("Bash");
        });

        it("ToolEnd clears", () => {
            const s0 = update(mk(), { type: "ToolStart", name: "Bash" }).state;
            const r = update(s0, { type: "ToolEnd" });
            expect(r.state.currentTool).toBe(null);
        });
    });

    describe("Tokens", () => {
        it("TokensIn / TokensOut accumulate independently", () => {
            const s0 = update(mk(), { type: "TokensIn", input: 50 }).state;
            const s1 = update(s0, { type: "TokensOut", output: 100 }).state;
            expect(s1.turnTokens).toEqual({ input: 50, output: 100 });
        });

        it("TokensIn preserves prior output", () => {
            const s0 = update(mk(), { type: "TokensOut", output: 100 }).state;
            const s1 = update(s0, { type: "TokensIn", input: 50 }).state;
            expect(s1.turnTokens).toEqual({ input: 50, output: 100 });
        });
    });

    describe("Stop flow", () => {
        it("RequestStop sets stopping", () => {
            const r = update(mk(), { type: "RequestStop", at: 100 });
            expect(r.state.stopping).toBe(true);
        });

        it("StopFailed clears stopping", () => {
            const s0 = update(mk(), { type: "RequestStop", at: 100 }).state;
            const r = update(s0, { type: "StopFailed" });
            expect(r.state.stopping).toBe(false);
        });

        it("TurnEnd clears stopping (the normal path — no explicit StopApplied needed)", () => {
            const s0 = update(mk(), { type: "StreamSubscribe", at: 100 }).state;
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, { type: "RequestStop", at: 120 }).state;
            const r = update(s2, { type: "TurnEnd", stats: null });
            expect(r.state.stopping).toBe(false);
        });
    });

    describe("Pending message FIFO", () => {
        it("Queue then accept removes the entry", () => {
            const s0 = update(mk(), {
                type: "PendingMessageQueued",
                id: "m1",
                text: "hi",
                at: 100,
            }).state;
            expect(s0.pending).toHaveLength(1);
            const r = update(s0, { type: "PendingMessageAccepted", id: "m1" });
            expect(r.state.pending).toHaveLength(0);
            expect(r.events[0]).toMatchObject({ type: "pending-accepted", wasPresent: true });
        });

        it("Accepting unknown id is idempotent no-op (with audit event)", () => {
            const start = mk();
            const r = update(start, { type: "PendingMessageAccepted", id: "ghost" });
            expect(r.state).toBe(start);
            expect(r.events[0]).toMatchObject({ type: "pending-accepted", wasPresent: false });
        });

        it("Reject removes the entry", () => {
            const s0 = update(mk(), {
                type: "PendingMessageQueued",
                id: "m1",
                text: "hi",
                at: 100,
            }).state;
            const r = update(s0, { type: "PendingMessageRejected", id: "m1" });
            expect(r.state.pending).toHaveLength(0);
        });

        it("Preserves FIFO order across multiple queues", () => {
            const s0 = update(mk(), {
                type: "PendingMessageQueued",
                id: "a",
                text: "1",
                at: 100,
            }).state;
            const s1 = update(s0, {
                type: "PendingMessageQueued",
                id: "b",
                text: "2",
                at: 110,
            }).state;
            const s2 = update(s1, {
                type: "PendingMessageQueued",
                id: "c",
                text: "3",
                at: 120,
            }).state;
            expect(s2.pending.map((m) => m.id)).toEqual(["a", "b", "c"]);
            const r = update(s2, { type: "PendingMessageAccepted", id: "b" });
            expect(r.state.pending.map((m) => m.id)).toEqual(["a", "c"]);
        });
    });

    describe("Purity", () => {
        it("does not mutate input state", () => {
            const start = update(mk(), { type: "StreamSubscribe", at: 100 }).state;
            const snapshot = JSON.parse(JSON.stringify(start));
            update(start, { type: "TurnStart", at: 110 });
            expect(start).toEqual(snapshot);
        });
    });
});
