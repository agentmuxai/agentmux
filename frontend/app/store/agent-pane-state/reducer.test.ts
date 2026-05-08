// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { update } from "./reducer";
import { initialState, STUCK_THRESHOLD_MS } from "./types";

const mk = () => initialState("test-agent");
/**
 * Convenience: bring a fresh state up to the "ready + subscribed" baseline
 * that most turn-related tests assume. Issue #728 introduced an init-phase
 * gate, so subscribing alone no longer permits `TurnStart`.
 */
const ready = (atMs = 100) => {
    const s0 = update(mk(), { type: "InitReady" }).state;
    return update(s0, { type: "StreamSubscribe", at: atMs }).state;
};

describe("agent-pane-state reducer", () => {
    describe("Stream lifecycle", () => {
        it("StreamSubscribe sets active + lastEventTime", () => {
            const r = update(mk(), { type: "StreamSubscribe", at: 100 });
            expect(r.state.streaming.active).toBe(true);
            expect(r.state.streaming.lastEventTime).toBe(100);
            expect(r.events[0]).toMatchObject({ type: "stream-subscribed", at: 100 });
        });

        it("StreamUnsubscribe clears active and force-clears turnActive", () => {
            const s0 = ready(100);
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
            const s0 = ready(100);
            const sWithStats = { ...s0, sessionStats: { input_tokens: 50, output_tokens: 100 } };
            const r = update(sWithStats, { type: "TurnStart", at: 110 });
            expect(r.state.turnActive).toBe(true);
            expect(r.state.sessionStats).toBe(null);
        });

        it("TurnEnd clears tool/tokens/turnActive AND stopping in one shot", () => {
            const s0 = ready(100);
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
            const s0 = ready(100);
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
            const s0 = ready(100);
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
            const s0 = ready(100);
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

    describe("Init phase (gap 1)", () => {
        it("starts in 'loading'", () => {
            expect(mk().initPhase).toBe("loading");
            expect(mk().initError).toBe(null);
        });

        it("InitReady advances to ready and clears error", () => {
            const s0 = update(mk(), { type: "InitFailed", reason: "boom" }).state;
            expect(s0.initPhase).toBe("error");
            expect(s0.initError).toBe("boom");
            const r = update(s0, { type: "InitReady" });
            expect(r.state.initPhase).toBe("ready");
            expect(r.state.initError).toBe(null);
            expect(r.events[0]).toMatchObject({ type: "init-ready" });
        });

        it("InitFailed captures reason", () => {
            const r = update(mk(), { type: "InitFailed", reason: "RPC timeout" });
            expect(r.state.initPhase).toBe("error");
            expect(r.state.initError).toBe("RPC timeout");
            expect(r.events[0]).toMatchObject({ type: "init-failed", reason: "RPC timeout" });
        });

        it("InitStart while already loading is a no-op (same ref)", () => {
            const start = mk();
            const r = update(start, { type: "InitStart" });
            expect(r.state).toBe(start);
            expect(r.events).toEqual([]);
        });

        it("TurnStart suppressed while initPhase === 'loading'", () => {
            // Subscribed but not InitReady — gap 1 invariant.
            const s0 = update(mk(), { type: "StreamSubscribe", at: 100 }).state;
            const r = update(s0, { type: "TurnStart", at: 110 });
            expect(r.state.turnActive).toBe(false);
            expect(r.events[0]).toMatchObject({
                type: "turn-start-suppressed",
                reason: "init still loading",
            });
        });

        it("TurnStart permitted on init error (fail-open)", () => {
            const s0 = update(mk(), { type: "InitFailed", reason: "load failed" }).state;
            const s1 = update(s0, { type: "StreamSubscribe", at: 100 }).state;
            const r = update(s1, { type: "TurnStart", at: 110 });
            expect(r.state.turnActive).toBe(true);
        });
    });

    describe("Stream watchdog (gap 3)", () => {
        it("StreamWatchdogTick is no-op when stream inactive", () => {
            const start = mk();
            const r = update(start, { type: "StreamWatchdogTick", nowMs: 100_000 });
            expect(r.state).toBe(start);
            expect(r.events).toEqual([]);
        });

        it("StreamWatchdogTick is no-op when no event seen yet", () => {
            // StreamSubscribe sets lastEventMs, so manually clear it to
            // exercise the "stream active but no events" branch.
            const s0 = update(mk(), { type: "StreamSubscribe", at: 100 }).state;
            const cleared = { ...s0, lastEventMs: null };
            const r = update(cleared, { type: "StreamWatchdogTick", nowMs: 200_000 });
            expect(r.state).toBe(cleared);
            expect(r.events).toEqual([]);
        });

        it("StreamWatchdogTick below threshold is silent", () => {
            const s0 = ready(1_000); // lastEventMs = 1000 from StreamSubscribe
            const r = update(s0, { type: "StreamWatchdogTick", nowMs: 10_000 });
            expect(r.events).toEqual([]);
        });

        it("StreamWatchdogTick at threshold emits stream-stuck", () => {
            const s0 = ready(0);
            const tick = STUCK_THRESHOLD_MS + 1_000;
            const r = update(s0, { type: "StreamWatchdogTick", nowMs: tick });
            expect(r.events).toHaveLength(1);
            expect(r.events[0]).toMatchObject({
                type: "stream-stuck",
                idleSinceMs: tick,
                thresholdMs: STUCK_THRESHOLD_MS,
            });
            // Watchdog never mutates state.
            expect(r.state).toBe(s0);
        });

        it("ToolStart bumps lastEventMs (resets watchdog clock)", () => {
            const s0 = ready(1_000);
            const r = update(s0, { type: "ToolStart", name: "Read" }, 5_000);
            expect(r.state.lastEventMs).toBe(5_000);
        });
    });

    describe("Pending message expiry (gap 2)", () => {
        it("PendingMessageExpired removes the entry by id", () => {
            const s0 = update(mk(), {
                type: "PendingMessageQueued",
                id: "x",
                text: "hi",
                at: 1_000,
            }).state;
            const r = update(s0, { type: "PendingMessageExpired", id: "x" }, 31_000);
            expect(r.state.pending).toHaveLength(0);
            expect(r.events[0]).toMatchObject({
                type: "pending-expired",
                id: "x",
                queuedAt: 1_000,
                ageMs: 30_000,
                wasPresent: true,
            });
        });

        it("PendingMessageExpired for unknown id is idempotent no-op", () => {
            const start = mk();
            const r = update(start, { type: "PendingMessageExpired", id: "ghost" });
            expect(r.state).toBe(start);
            expect(r.events[0]).toMatchObject({
                type: "pending-expired",
                id: "ghost",
                wasPresent: false,
            });
        });

        it("PendingMessageExpired after Accepted is harmless (already removed)", () => {
            const s0 = update(mk(), {
                type: "PendingMessageQueued",
                id: "y",
                text: "hi",
                at: 100,
            }).state;
            const s1 = update(s0, { type: "PendingMessageAccepted", id: "y" }).state;
            // Race: timeout fires after acceptance.
            const r = update(s1, { type: "PendingMessageExpired", id: "y" });
            expect(r.state.pending).toHaveLength(0);
            expect(r.events[0]).toMatchObject({
                type: "pending-expired",
                wasPresent: false,
            });
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
