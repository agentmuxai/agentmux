// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { update } from "./reducer";
import {
    AgentPaneState,
    initialState,
    isWorking,
    STUCK_THRESHOLD_MS,
    TurnPhase,
} from "./types";

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

    // ─────────────────────────────────────────────────────────────────
    // PR A — TurnPhase dual-write
    //
    // Spec: docs/specs/SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md §5.
    // Every command that mutates legacy {turnActive, stopping,
    // streaming.active} must also write the corresponding turnPhase
    // kind. These tests pin the dual-write invariant — legacy + new
    // fields agree.
    // ─────────────────────────────────────────────────────────────────
    describe("TurnPhase dual-write (PR A)", () => {
        it("initialState starts in turnPhase Idle", () => {
            const s = mk();
            expect(s.turnPhase).toEqual({ kind: "Idle" });
            // Legacy invariant: nothing is "working" yet.
            expect(s.turnActive).toBe(false);
            expect(s.stopping).toBe(false);
            expect(s.streaming.active).toBe(false);
        });

        it("TurnStart sets phase to Submitting AND turnActive=true", () => {
            const s0 = ready(100);
            const r = update(s0, { type: "TurnStart", at: 110 });
            // Legacy
            expect(r.state.turnActive).toBe(true);
            // New phase
            expect(r.state.turnPhase.kind).toBe("Submitting");
            if (r.state.turnPhase.kind === "Submitting") {
                expect(r.state.turnPhase.submittedAt).toBe(110);
                expect(r.state.turnPhase.pendingContent).toBe("");
            }
        });

        it("Suppressed TurnStart does NOT advance turnPhase", () => {
            // Stream inactive → suppressed; phase stays Idle.
            const start = mk();
            const r = update(start, { type: "TurnStart", at: 100 });
            expect(r.state.turnPhase.kind).toBe("Idle");
            expect(r.state.turnActive).toBe(false);
        });

        it("StreamSubscribe after Submitting advances phase to Streaming", () => {
            // Manual sequence: Submitting then a fresh subscribe (e.g.
            // after reconnect). The subscribe should hand off to
            // Streaming.
            const s0 = update(mk(), { type: "InitReady" }).state;
            const s1 = update(s0, { type: "StreamSubscribe", at: 100 }).state;
            const s2 = update(s1, { type: "TurnStart", at: 110 }).state;
            expect(s2.turnPhase.kind).toBe("Submitting");
            // Re-subscribe (simulates the recovery handshake).
            const r = update(s2, { type: "StreamSubscribe", at: 120 });
            expect(r.state.turnPhase.kind).toBe("Streaming");
            if (r.state.turnPhase.kind === "Streaming") {
                expect(r.state.turnPhase.toolsActive).toBe(0);
                expect(r.state.turnPhase.lastEventMs).toBe(120);
            }
            // Legacy still consistent.
            expect(r.state.streaming.active).toBe(true);
            expect(r.state.turnActive).toBe(true);
        });

        it("StreamSubscribe from Idle keeps phase Idle (no spontaneous promotion)", () => {
            const s0 = update(mk(), { type: "InitReady" }).state;
            const r = update(s0, { type: "StreamSubscribe", at: 100 });
            expect(r.state.streaming.active).toBe(true);
            expect(r.state.turnPhase.kind).toBe("Idle");
        });

        it("StreamFlushObserved while Streaming mirrors bufferSize + lastEventMs onto phase", () => {
            const s0 = update(mk(), { type: "InitReady" }).state;
            const s1 = update(s0, { type: "StreamSubscribe", at: 100 }).state;
            const s2 = update(s1, { type: "TurnStart", at: 110 }).state;
            const s3 = update(s2, { type: "StreamSubscribe", at: 120 }).state;
            expect(s3.turnPhase.kind).toBe("Streaming");
            const r = update(s3, {
                type: "StreamFlushObserved",
                addedCount: 5,
                at: 130,
            });
            expect(r.state.streaming.bufferSize).toBe(5);
            expect(r.state.turnPhase.kind).toBe("Streaming");
            if (r.state.turnPhase.kind === "Streaming") {
                expect(r.state.turnPhase.bufferSize).toBe(5);
                expect(r.state.turnPhase.lastEventMs).toBe(130);
            }
        });

        it("TurnEnd transitions phase to Done.completed AND clears turnActive/stopping", () => {
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const r = update(s1, { type: "TurnEnd", stats: null }, 200);
            // Legacy
            expect(r.state.turnActive).toBe(false);
            expect(r.state.stopping).toBe(false);
            // New phase
            expect(r.state.turnPhase.kind).toBe("Done");
            if (r.state.turnPhase.kind === "Done") {
                expect(r.state.turnPhase.outcome).toBe("completed");
                expect(r.state.turnPhase.finishedAt).toBe(200);
            }
        });

        it("TurnEnd while stopping was set produces Done.stopped", () => {
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, { type: "RequestStop", at: 115 }).state;
            const r = update(s2, { type: "TurnEnd", stats: null }, 200);
            expect(r.state.turnPhase.kind).toBe("Done");
            if (r.state.turnPhase.kind === "Done") {
                expect(r.state.turnPhase.outcome).toBe("stopped");
            }
            expect(r.state.stopping).toBe(false);
            expect(r.state.turnActive).toBe(false);
        });

        it("TurnReset returns phase to Idle AND clears turnActive/stopping", () => {
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, { type: "RequestStop", at: 115 }).state;
            const r = update(s2, { type: "TurnReset" });
            expect(r.state.turnPhase).toEqual({ kind: "Idle" });
            expect(r.state.turnActive).toBe(false);
            expect(r.state.stopping).toBe(false);
        });

        it("RequestStop while working sets phase to Interrupting AND stopping=true", () => {
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const r = update(s1, { type: "RequestStop", at: 120 });
            // Legacy
            expect(r.state.stopping).toBe(true);
            // New phase
            expect(r.state.turnPhase.kind).toBe("Interrupting");
            if (r.state.turnPhase.kind === "Interrupting") {
                expect(r.state.turnPhase.reason).toBe("user");
                expect(r.state.turnPhase.sigintSentAt).toBe(120);
            }
        });

        it("RequestStop while Idle sets legacy stopping but leaves phase Idle", () => {
            // Edge case: stop pressed without a turn in flight. The
            // legacy boolean still flips; the phase is unaffected
            // (there's no working state to interrupt).
            const start = mk();
            const r = update(start, { type: "RequestStop", at: 100 });
            expect(r.state.stopping).toBe(true);
            expect(r.state.turnPhase.kind).toBe("Idle");
        });

        it("StopFailed while Interrupting rolls back to Streaming AND clears stopping", () => {
            // Build up: ready → submit → re-subscribe to Streaming →
            // stop → stop-rpc-fails.
            const s0 = update(mk(), { type: "InitReady" }).state;
            const s1 = update(s0, { type: "StreamSubscribe", at: 100 }).state;
            const s2 = update(s1, { type: "TurnStart", at: 110 }).state;
            const s3 = update(s2, { type: "StreamSubscribe", at: 120 }).state;
            expect(s3.turnPhase.kind).toBe("Streaming");
            const s4 = update(s3, { type: "RequestStop", at: 130 }).state;
            expect(s4.turnPhase.kind).toBe("Interrupting");
            const r = update(s4, { type: "StopFailed" });
            // Legacy
            expect(r.state.stopping).toBe(false);
            // New phase — rolled back because stream is still active.
            expect(r.state.turnPhase.kind).toBe("Streaming");
        });

        it("StopFailed when not Interrupting just clears stopping (phase unchanged)", () => {
            const start = mk();
            const flagged: AgentPaneState = { ...start, stopping: true };
            const r = update(flagged, { type: "StopFailed" });
            expect(r.state.stopping).toBe(false);
            // Phase was Idle and remains Idle.
            expect(r.state.turnPhase.kind).toBe("Idle");
        });

        it("StreamUnsubscribe while working surfaces Disconnected.{lastKind}", () => {
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            // Phase is Submitting (legacy turnActive=true).
            expect(s1.turnPhase.kind).toBe("Submitting");
            const r = update(s1, { type: "StreamUnsubscribe", at: 200 });
            // Legacy: stream + turnActive cleared.
            expect(r.state.streaming.active).toBe(false);
            expect(r.state.turnActive).toBe(false);
            // New: Disconnected, with Submitting as the lost kind.
            expect(r.state.turnPhase.kind).toBe("Disconnected");
            if (r.state.turnPhase.kind === "Disconnected") {
                expect(r.state.turnPhase.lastKind).toBe("Submitting");
                expect(r.state.turnPhase.reason).toBe("stream-unsubscribed");
            }
        });

        it("StreamUnsubscribe while Idle returns to Idle (no Disconnected)", () => {
            const s0 = update(mk(), { type: "InitReady" }).state;
            const s1 = update(s0, { type: "StreamSubscribe", at: 100 }).state;
            // Phase is still Idle (subscribe alone doesn't promote).
            expect(s1.turnPhase.kind).toBe("Idle");
            const r = update(s1, { type: "StreamUnsubscribe", at: 200 });
            expect(r.state.turnPhase.kind).toBe("Idle");
            expect(r.state.streaming.active).toBe(false);
        });

        it("StreamFlushObserved while Submitting promotes phase to Streaming (codex P1 on #987)", () => {
            // Realistic runtime flow: subscribe ONCE at mount, then each
            // user message dispatches TurnStart. Submitting → Streaming
            // must promote on the first chunk arrival, NOT depend on a
            // re-subscribe that never fires in practice.
            const s0 = update(mk(), { type: "InitReady" }).state;
            const s1 = update(s0, { type: "StreamSubscribe", at: 100 }).state;
            // (no second subscribe — that was the synthetic test crutch)
            const s2 = update(s1, { type: "TurnStart", at: 110 }).state;
            expect(s2.turnPhase.kind).toBe("Submitting");
            const r = update(s2, {
                type: "StreamFlushObserved",
                addedCount: 3,
                at: 120,
            });
            expect(r.state.turnPhase.kind).toBe("Streaming");
            if (r.state.turnPhase.kind === "Streaming") {
                expect(r.state.turnPhase.bufferSize).toBe(3);
                expect(r.state.turnPhase.toolsActive).toBe(0);
                expect(r.state.turnPhase.lastEventMs).toBe(120);
            }
        });

        it("ToolStart while Submitting promotes phase to Streaming with toolsActive=1", () => {
            const s0 = update(mk(), { type: "InitReady" }).state;
            const s1 = update(s0, { type: "StreamSubscribe", at: 100 }).state;
            const s2 = update(s1, { type: "TurnStart", at: 110 }).state;
            expect(s2.turnPhase.kind).toBe("Submitting");
            const r = update(s2, { type: "ToolStart", name: "Read" }, 130);
            expect(r.state.turnPhase.kind).toBe("Streaming");
            if (r.state.turnPhase.kind === "Streaming") {
                expect(r.state.turnPhase.toolsActive).toBe(1);
                expect(r.state.turnPhase.lastEventMs).toBe(130);
            }
        });

        it("TokensIn while Submitting promotes phase to Streaming", () => {
            const s0 = update(mk(), { type: "InitReady" }).state;
            const s1 = update(s0, { type: "StreamSubscribe", at: 100 }).state;
            const s2 = update(s1, { type: "TurnStart", at: 110 }).state;
            expect(s2.turnPhase.kind).toBe("Submitting");
            const r = update(s2, { type: "TokensIn", input: 50 }, 200);
            expect(r.state.turnPhase.kind).toBe("Streaming");
            if (r.state.turnPhase.kind === "Streaming") {
                expect(r.state.turnPhase.lastEventMs).toBe(200);
                expect(r.state.turnPhase.toolsActive).toBe(0);
            }
        });

        it("ToolStart while Streaming increments toolsActive on the phase", () => {
            const s0 = update(mk(), { type: "InitReady" }).state;
            const s1 = update(s0, { type: "StreamSubscribe", at: 100 }).state;
            const s2 = update(s1, { type: "TurnStart", at: 110 }).state;
            const s3 = update(s2, { type: "StreamSubscribe", at: 120 }).state;
            const s4 = update(s3, { type: "ToolStart", name: "Read" }, 130).state;
            expect(s4.currentTool).toBe("Read");
            expect(s4.turnPhase.kind).toBe("Streaming");
            if (s4.turnPhase.kind === "Streaming") {
                expect(s4.turnPhase.toolsActive).toBe(1);
                expect(s4.turnPhase.lastEventMs).toBe(130);
            }
            const s5 = update(s4, { type: "ToolStart", name: "Edit" }, 140).state;
            if (s5.turnPhase.kind === "Streaming") {
                expect(s5.turnPhase.toolsActive).toBe(2);
            }
            const s6 = update(s5, { type: "ToolEnd" }, 150).state;
            expect(s6.currentTool).toBe(null);
            if (s6.turnPhase.kind === "Streaming") {
                expect(s6.turnPhase.toolsActive).toBe(1);
                expect(s6.turnPhase.lastEventMs).toBe(150);
            }
        });

        it("ToolEnd clamps toolsActive at 0 (never goes negative)", () => {
            // Defensive: out-of-order ToolEnd while toolsActive is 0.
            const s0 = update(mk(), { type: "InitReady" }).state;
            const s1 = update(s0, { type: "StreamSubscribe", at: 100 }).state;
            const s2 = update(s1, { type: "TurnStart", at: 110 }).state;
            const s3 = update(s2, { type: "StreamSubscribe", at: 120 }).state;
            expect(s3.turnPhase.kind).toBe("Streaming");
            const r = update(s3, { type: "ToolEnd" }, 130);
            if (r.state.turnPhase.kind === "Streaming") {
                expect(r.state.turnPhase.toolsActive).toBe(0);
            }
        });

        it("TokensIn / TokensOut while Streaming bump lastEventMs on the phase", () => {
            const s0 = update(mk(), { type: "InitReady" }).state;
            const s1 = update(s0, { type: "StreamSubscribe", at: 100 }).state;
            const s2 = update(s1, { type: "TurnStart", at: 110 }).state;
            const s3 = update(s2, { type: "StreamSubscribe", at: 120 }).state;
            const s4 = update(s3, { type: "TokensIn", input: 50 }, 200).state;
            if (s4.turnPhase.kind === "Streaming") {
                expect(s4.turnPhase.lastEventMs).toBe(200);
            }
            const s5 = update(s4, { type: "TokensOut", output: 100 }, 210).state;
            if (s5.turnPhase.kind === "Streaming") {
                expect(s5.turnPhase.lastEventMs).toBe(210);
            }
        });

        it("Pending queue commands do NOT touch turnPhase", () => {
            // pending[] is composer-side; it's orthogonal to the turn
            // lifecycle. PR A keeps it that way.
            const s0 = ready(100);
            const s1 = update(s0, {
                type: "PendingMessageQueued",
                id: "m1",
                text: "hi",
                at: 105,
            }).state;
            expect(s1.turnPhase.kind).toBe("Idle");
            const s2 = update(s1, { type: "PendingMessageAccepted", id: "m1" }).state;
            expect(s2.turnPhase.kind).toBe("Idle");
            const s3 = update(s2, {
                type: "PendingMessageQueued",
                id: "m2",
                text: "hi2",
                at: 200,
            }).state;
            const s4 = update(s3, { type: "PendingMessageRejected", id: "m2" }).state;
            expect(s4.turnPhase.kind).toBe("Idle");
            const s5 = update(s4, {
                type: "PendingMessageQueued",
                id: "m3",
                text: "hi3",
                at: 300,
            }).state;
            const s6 = update(s5, { type: "PendingMessageExpired", id: "m3" }, 1000).state;
            expect(s6.turnPhase.kind).toBe("Idle");
        });

        it("Init lifecycle commands do NOT touch turnPhase", () => {
            // initPhase and turnPhase are orthogonal axes.
            const s0 = update(mk(), { type: "InitStart" }).state;
            expect(s0.turnPhase.kind).toBe("Idle");
            const s1 = update(s0, { type: "InitReady" }).state;
            expect(s1.turnPhase.kind).toBe("Idle");
            const s2 = update(s1, { type: "InitFailed", reason: "boom" }).state;
            expect(s2.turnPhase.kind).toBe("Idle");
        });

        it("StreamWatchdogTick does NOT mutate turnPhase", () => {
            const s0 = ready(0);
            const tick = STUCK_THRESHOLD_MS + 1_000;
            const r = update(s0, { type: "StreamWatchdogTick", nowMs: tick });
            // Watchdog is event-only, never mutates.
            expect(r.state).toBe(s0);
            expect(r.state.turnPhase.kind).toBe("Idle");
        });

        it("dual-write invariant: legacy.turnActive ↔ phase.kind ∈ working set", () => {
            // Walk a normal turn and assert the invariant at every step.
            const s0 = ready(100); // Idle, !turnActive
            expect(s0.turnActive).toBe(false);
            expect(workingByLegacy(s0)).toBe(false);
            expect(isWorking(s0)).toBe(false);

            const s1 = update(s0, { type: "TurnStart", at: 110 }).state; // Submitting, turnActive
            expect(s1.turnActive).toBe(true);
            expect(workingByLegacy(s1)).toBe(true);
            expect(isWorking(s1)).toBe(true);

            const s2 = update(s1, { type: "StreamSubscribe", at: 120 }).state; // Streaming
            expect(s2.turnActive).toBe(true);
            expect(isWorking(s2)).toBe(true);

            const s3 = update(s2, { type: "RequestStop", at: 130 }).state; // Interrupting
            expect(s3.stopping).toBe(true);
            expect(isWorking(s3)).toBe(true);

            const s4 = update(s3, { type: "TurnEnd", stats: null }, 140).state; // Done
            expect(s4.turnActive).toBe(false);
            expect(s4.stopping).toBe(false);
            expect(workingByLegacy(s4)).toBe(false);
            expect(isWorking(s4)).toBe(false);
        });
    });

    // ─────────────────────────────────────────────────────────────────
    // isWorking selector — spec §7.
    // ─────────────────────────────────────────────────────────────────
    describe("isWorking selector", () => {
        const stateWith = (phase: TurnPhase): AgentPaneState => ({
            ...mk(),
            turnPhase: phase,
        });

        const matrix: Array<[TurnPhase, boolean]> = [
            [{ kind: "Idle" }, false],
            [
                {
                    kind: "Submitting",
                    submittedAt: 0,
                    pendingContent: "hi",
                },
                true,
            ],
            [
                {
                    kind: "Streaming",
                    bufferSize: 0,
                    toolsActive: 0,
                    lastEventMs: 0,
                },
                true,
            ],
            [
                { kind: "Interrupting", reason: "user", sigintSentAt: 0 },
                true,
            ],
            [{ kind: "Done", outcome: "completed", finishedAt: 0 }, false],
            [
                {
                    kind: "Disconnected",
                    lastKind: "Streaming",
                    reason: "stream-unsubscribed",
                },
                false,
            ],
        ];

        for (const [phase, expected] of matrix) {
            it(`${phase.kind} → ${expected}`, () => {
                expect(isWorking(stateWith(phase))).toBe(expected);
            });
        }
    });
});

/**
 * Convenience: the "working" predicate as derived from the legacy
 * booleans only. Used to pin the dual-write invariant — must agree
 * with `isWorking(state)` (which reads turnPhase). PR G removes this
 * once the legacy fields are gone.
 */
function workingByLegacy(state: AgentPaneState): boolean {
    return state.turnActive || state.stopping;
}
