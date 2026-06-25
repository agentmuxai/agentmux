// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { update } from "./reducer";
import {
    AgentPaneState,
    initialState,
    INTERRUPT_TIMEOUT_MS,
    isDisconnected,
    isInitReady,
    isWorking,
    STUCK_THRESHOLD_MS,
    SUBMIT_TIMEOUT_MS,
    TurnPhase,
} from "./types";

const mk = () => initialState("test-agent");
/**
 * Convenience: bring a fresh state up to the "ready + subscribed" baseline
 * that most turn-related tests assume. Issue #728 introduced an init-phase
 * gate, so subscribing alone no longer permits `TurnStart`.
 */
const ready = (atMs = 100) => {
    const s0 = update(mk(), { type: "InitReady", at: atMs }).state;
    return update(s0, { type: "StreamSubscribe", at: atMs }).state;
};

describe("agent-pane-state reducer", () => {
    describe("Stream lifecycle", () => {
        it("StreamSubscribe sets lastEventMs + streaming telemetry", () => {
            const r = update(mk(), { type: "StreamSubscribe", at: 100 });
            // PR G: subscribed-ness is `lastEventMs !== null`; the
            // legacy `streaming.active` boolean was dropped.
            expect(r.state.lastEventMs).toBe(100);
            expect(r.state.streaming.lastEventTime).toBe(100);
            expect(r.events[0]).toMatchObject({ type: "stream-subscribed", at: 100 });
        });

        it("StreamUnsubscribe clears subscription and forces working turn into Disconnected", () => {
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            expect(isWorking(s1)).toBe(true);
            const r = update(s1, { type: "StreamUnsubscribe", at: 200 });
            expect(r.state.lastEventMs).toBe(null);
            expect(isWorking(r.state)).toBe(false);
            expect(r.state.turnPhase.kind).toBe("Disconnected");
        });

        it("StreamFlushObserved bumps bufferSize when subscribed", () => {
            const s0 = update(mk(), { type: "StreamSubscribe", at: 100 }).state;
            const r = update(s0, { type: "StreamFlushObserved", addedCount: 3, at: 110 });
            expect(r.state.streaming.bufferSize).toBe(3);
            expect(r.state.streaming.lastEventTime).toBe(110);
        });

        it("StreamFlushObserved is no-op when stream unsubscribed", () => {
            const start = mk();
            const r = update(start, { type: "StreamFlushObserved", addedCount: 3, at: 110 });
            // Reducer must return SAME reference when no work was done.
            expect(r.state).toBe(start);
            expect(r.events).toEqual([]);
        });
    });

    describe("Turn lifecycle invariants", () => {
        it("TurnStart while stream unsubscribed is suppressed", () => {
            const start = mk();
            const r = update(start, { type: "TurnStart", at: 100 });
            expect(r.state.turnPhase.kind).toBe("Idle");
            expect(r.state).toBe(start);
            expect(r.events[0]).toMatchObject({ type: "turn-start-suppressed" });
        });

        it("TurnStart while subscribed transitions to Submitting + clears stale stats", () => {
            const s0 = ready(100);
            const sWithStats = { ...s0, sessionStats: { input_tokens: 50, output_tokens: 100 } };
            const r = update(sWithStats, { type: "TurnStart", at: 110 });
            expect(r.state.turnPhase.kind).toBe("Submitting");
            expect(isWorking(r.state)).toBe(true);
            expect(r.state.sessionStats).toBe(null);
        });

        it("TurnEnd clears tool/tokens and lands in Done in one shot", () => {
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
            expect(isWorking(r.state)).toBe(false);
            expect(r.state.turnPhase.kind).toBe("Done");
            expect(r.state.currentTool).toBe(null);
            expect(r.state.turnTokens).toBe(null);
            // Stats merged from live tokens (mergeStats fallback path).
            expect(r.state.sessionStats).toEqual({ input_tokens: 50, output_tokens: 200 });
            expect(r.events[0]).toMatchObject({
                type: "turn-ended",
                // outcome is "stopped" because RequestStop put the phase
                // into Interrupting before TurnEnd ran. Sound subsystem
                // (SPEC_SOUND_NOTIFICATIONS_2026_06_05.md §3.2) reads
                // outcome directly from the event without snapshotting.
                outcome: "stopped",
                statsMerged: true,
                // stoppingCleared still carries the audit signal — true
                // iff the turn ended while in Interrupting (PR G:
                // derived from `turnPhase.kind === "Interrupting"`).
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

        it("TurnEnd prefers token-bearing result totals over live last-message tokens", () => {
            // Live turnTokens hold only the last message_start/message_delta
            // (TokensIn/TokensOut overwrite); the result carries the
            // cache-inclusive turn total, which must win.
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, { type: "TokensIn", input: 2 }).state;
            const s3 = update(s2, { type: "TokensOut", output: 300 }).state;
            const r = update(s3, {
                type: "TurnEnd",
                stats: { input_tokens: 70000, output_tokens: 512 } as any,
            });
            expect(r.state.sessionStats).toMatchObject({ input_tokens: 70000, output_tokens: 512 });
        });

        it("TurnReset clears turn-scoped state but keeps subscription + pending", () => {
            const s0 = ready(100);
            const s1 = update(s0, {
                type: "PendingMessageQueued", enqueuedWhileBusy: false,
                id: "p1",
                text: "hello",
                at: 105,
            }).state;
            const s2 = update(s1, { type: "TurnStart", at: 110 }).state;
            const s3 = update(s2, { type: "ToolStart", name: "Edit" }).state;
            const r = update(s3, { type: "TurnReset" });
            // PR G: subscription gate is `lastEventMs !== null` (was
            // `streaming.active`). Preserved across TurnReset.
            expect(r.state.lastEventMs).not.toBeNull();
            expect(r.state.pending).toHaveLength(1); // preserved
            expect(r.state.currentTool).toBe(null);
            expect(r.state.turnPhase.kind).toBe("Idle");
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
        it("RequestStop while working transitions to Interrupting", () => {
            // PR G: RequestStop is only meaningful while a turn is in
            // flight. Subscribe + start a turn first, then stop.
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const r = update(s1, { type: "RequestStop", at: 120 });
            expect(r.state.turnPhase.kind).toBe("Interrupting");
        });

        it("RequestStop while Idle is a state no-op (only emits audit event)", () => {
            // PR G: legacy `stopping` boolean used to flip true here
            // even with no turn in flight; that was a latent bug
            // surface (the stop could not actually be acted on).
            // Now it's a clean no-op state-wise.
            const start = mk();
            const r = update(start, { type: "RequestStop", at: 100 });
            expect(r.state).toBe(start);
            expect(r.events[0]).toMatchObject({ type: "stop-requested", at: 100 });
        });

        it("StopFailed clears Interrupting → Streaming (when subscribed)", () => {
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, { type: "RequestStop", at: 120 }).state;
            expect(s2.turnPhase.kind).toBe("Interrupting");
            const r = update(s2, { type: "StopFailed" });
            // Rolls back to Streaming since the stream is still subscribed.
            expect(r.state.turnPhase.kind).toBe("Streaming");
        });

        it("TurnEnd transitions Interrupting → Done.stopped (was the legacy 'stopping cleared' path)", () => {
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, { type: "RequestStop", at: 120 }).state;
            const r = update(s2, { type: "TurnEnd", stats: null });
            expect(r.state.turnPhase.kind).toBe("Done");
            if (r.state.turnPhase.kind === "Done") {
                expect(r.state.turnPhase.outcome).toBe("stopped");
            }
        });
    });

    describe("Pending message FIFO", () => {
        it("Queue then accept removes the entry", () => {
            const s0 = update(mk(), {
                type: "PendingMessageQueued", enqueuedWhileBusy: false,
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
                type: "PendingMessageQueued", enqueuedWhileBusy: false,
                id: "m1",
                text: "hi",
                at: 100,
            }).state;
            const r = update(s0, { type: "PendingMessageRejected", id: "m1" });
            expect(r.state.pending).toHaveLength(0);
        });

        it("Preserves FIFO order across multiple queues", () => {
            const s0 = update(mk(), {
                type: "PendingMessageQueued", enqueuedWhileBusy: false,
                id: "a",
                text: "1",
                at: 100,
            }).state;
            const s1 = update(s0, {
                type: "PendingMessageQueued", enqueuedWhileBusy: false,
                id: "b",
                text: "2",
                at: 110,
            }).state;
            const s2 = update(s1, {
                type: "PendingMessageQueued", enqueuedWhileBusy: false,
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
        it("starts in InitPending", () => {
            const s = mk();
            expect(s.initPhase).toEqual({ kind: "InitPending" });
            expect(isInitReady(s)).toBe(false);
        });

        it("InitReady advances InitPending → InitReady with timestamp", () => {
            const r = update(mk(), { type: "InitReady", at: 500 });
            expect(r.state.initPhase).toEqual({ kind: "InitReady", at: 500 });
            expect(isInitReady(r.state)).toBe(true);
            expect(r.events[0]).toMatchObject({ type: "init-ready" });
        });

        it("InitFailed advances InitPending → InitFailed with timestamp + reason", () => {
            const r = update(mk(), { type: "InitFailed", at: 750, reason: "RPC timeout" });
            expect(r.state.initPhase).toEqual({
                kind: "InitFailed",
                at: 750,
                reason: "RPC timeout",
            });
            expect(isInitReady(r.state)).toBe(false);
            expect(r.events[0]).toMatchObject({ type: "init-failed", reason: "RPC timeout" });
        });

        it("InitStart in InitPending is a no-op (same ref, no events)", () => {
            const start = mk();
            const r = update(start, { type: "InitStart" });
            expect(r.state).toBe(start);
            expect(r.events).toEqual([]);
        });

        it("InitStart after InitReady is a no-op (one-way; same ref)", () => {
            const readyState = update(mk(), { type: "InitReady", at: 100 }).state;
            const r = update(readyState, { type: "InitStart" });
            expect(r.state).toBe(readyState);
            expect(r.events).toEqual([]);
        });

        it("InitStart after InitFailed is a no-op (one-way; same ref)", () => {
            const failed = update(mk(), {
                type: "InitFailed",
                at: 100,
                reason: "boom",
            }).state;
            const r = update(failed, { type: "InitStart" });
            expect(r.state).toBe(failed);
            expect(r.events).toEqual([]);
        });

        it("InitReady after InitReady is a no-op (idempotent)", () => {
            const s0 = update(mk(), { type: "InitReady", at: 100 }).state;
            const r = update(s0, { type: "InitReady", at: 200 });
            expect(r.state).toBe(s0);
            // Original timestamp is preserved — re-firing doesn't bump it.
            expect(r.state.initPhase).toEqual({ kind: "InitReady", at: 100 });
            expect(r.events).toEqual([]);
        });

        it("InitReady after InitFailed is dropped (one-way; same ref)", () => {
            const failed = update(mk(), {
                type: "InitFailed",
                at: 100,
                reason: "load broke",
            }).state;
            const r = update(failed, { type: "InitReady", at: 200 });
            expect(r.state).toBe(failed);
            // Stays in InitFailed — reason preserved.
            expect(r.state.initPhase).toEqual({
                kind: "InitFailed",
                at: 100,
                reason: "load broke",
            });
            expect(r.events).toEqual([]);
        });

        it("InitFailed after InitReady is dropped (one-way; same ref)", () => {
            const readyState = update(mk(), { type: "InitReady", at: 100 }).state;
            const r = update(readyState, {
                type: "InitFailed",
                at: 200,
                reason: "late",
            });
            expect(r.state).toBe(readyState);
            expect(r.state.initPhase).toEqual({ kind: "InitReady", at: 100 });
            expect(r.events).toEqual([]);
        });

        it("InitFailed after InitFailed is a no-op (preserves first reason + timestamp)", () => {
            const first = update(mk(), {
                type: "InitFailed",
                at: 100,
                reason: "first failure",
            }).state;
            const r = update(first, {
                type: "InitFailed",
                at: 200,
                reason: "second failure",
            });
            expect(r.state).toBe(first);
            expect(r.state.initPhase).toEqual({
                kind: "InitFailed",
                at: 100,
                reason: "first failure",
            });
            expect(r.events).toEqual([]);
        });

        it("TurnStart suppressed while in InitPending", () => {
            // Subscribed but not InitReady — gap 1 invariant.
            const s0 = update(mk(), { type: "StreamSubscribe", at: 100 }).state;
            const r = update(s0, { type: "TurnStart", at: 110 });
            expect(r.state.turnPhase.kind).toBe("Idle");
            expect(r.events[0]).toMatchObject({
                type: "turn-start-suppressed",
                reason: "init still loading",
            });
        });

        it("TurnStart permitted on InitFailed (fail-open)", () => {
            const s0 = update(mk(), {
                type: "InitFailed",
                at: 100,
                reason: "load failed",
            }).state;
            const s1 = update(s0, { type: "StreamSubscribe", at: 100 }).state;
            const r = update(s1, { type: "TurnStart", at: 110 });
            expect(r.state.turnPhase.kind).toBe("Submitting");
            expect(isWorking(r.state)).toBe(true);
        });

        it("isInitReady selector tracks InitReady only", () => {
            expect(isInitReady(mk())).toBe(false);
            const failed = update(mk(), {
                type: "InitFailed",
                at: 100,
                reason: "x",
            }).state;
            expect(isInitReady(failed)).toBe(false);
            const readyState = update(mk(), { type: "InitReady", at: 100 }).state;
            expect(isInitReady(readyState)).toBe(true);
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
                type: "PendingMessageQueued", enqueuedWhileBusy: false,
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
                type: "PendingMessageQueued", enqueuedWhileBusy: false,
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
    // TurnPhase transitions — single source of truth (since PR G).
    //
    // Spec: docs/specs/SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md §5.
    // PR A introduced dual-write against legacy {turnActive, stopping,
    // streaming.active}; PR B migrated views; PR G removed the legacy
    // fields. These tests now pin the phase transitions on their own.
    // ─────────────────────────────────────────────────────────────────
    describe("TurnPhase transitions", () => {
        it("initialState starts in turnPhase Idle (and not subscribed)", () => {
            const s = mk();
            expect(s.turnPhase).toEqual({ kind: "Idle" });
            expect(isWorking(s)).toBe(false);
            expect(s.lastEventMs).toBeNull();
        });

        it("TurnStart sets phase to Submitting", () => {
            const s0 = ready(100);
            const r = update(s0, { type: "TurnStart", at: 110 });
            expect(r.state.turnPhase.kind).toBe("Submitting");
            expect(isWorking(r.state)).toBe(true);
            if (r.state.turnPhase.kind === "Submitting") {
                expect(r.state.turnPhase.submittedAt).toBe(110);
                expect(r.state.turnPhase.pendingContent).toBe("");
            }
        });

        it("Suppressed TurnStart does NOT advance turnPhase", () => {
            // Stream unsubscribed → suppressed; phase stays Idle.
            const start = mk();
            const r = update(start, { type: "TurnStart", at: 100 });
            expect(r.state.turnPhase.kind).toBe("Idle");
            expect(isWorking(r.state)).toBe(false);
        });

        it("StreamSubscribe after Submitting advances phase to Streaming", () => {
            // Manual sequence: Submitting then a fresh subscribe (e.g.
            // after reconnect). The subscribe should hand off to
            // Streaming.
            const s0 = update(mk(), { type: "InitReady", at: 100 }).state;
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
            // Subscription gate (PR G: replaces `streaming.active`).
            expect(r.state.lastEventMs).toBe(120);
            expect(isWorking(r.state)).toBe(true);
        });

        it("StreamSubscribe from Idle keeps phase Idle (no spontaneous promotion)", () => {
            const s0 = update(mk(), { type: "InitReady", at: 100 }).state;
            const r = update(s0, { type: "StreamSubscribe", at: 100 });
            expect(r.state.lastEventMs).toBe(100);
            expect(r.state.turnPhase.kind).toBe("Idle");
        });

        it("StreamFlushObserved while Streaming mirrors bufferSize + lastEventMs onto phase", () => {
            const s0 = update(mk(), { type: "InitReady", at: 100 }).state;
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

        it("TurnEnd transitions phase to Done.completed", () => {
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const r = update(s1, { type: "TurnEnd", stats: null }, 200);
            expect(isWorking(r.state)).toBe(false);
            expect(r.state.turnPhase.kind).toBe("Done");
            if (r.state.turnPhase.kind === "Done") {
                expect(r.state.turnPhase.outcome).toBe("completed");
                expect(r.state.turnPhase.finishedAt).toBe(200);
            }
        });

        it("TurnEnd while in Interrupting produces Done.stopped", () => {
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, { type: "RequestStop", at: 115 }).state;
            const r = update(s2, { type: "TurnEnd", stats: null }, 200);
            expect(r.state.turnPhase.kind).toBe("Done");
            if (r.state.turnPhase.kind === "Done") {
                expect(r.state.turnPhase.outcome).toBe("stopped");
            }
            expect(isWorking(r.state)).toBe(false);
        });

        it("TurnReset returns phase to Idle", () => {
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, { type: "RequestStop", at: 115 }).state;
            const r = update(s2, { type: "TurnReset" });
            expect(r.state.turnPhase).toEqual({ kind: "Idle" });
            expect(isWorking(r.state)).toBe(false);
        });

        it("RequestStop while working sets phase to Interrupting", () => {
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const r = update(s1, { type: "RequestStop", at: 120 });
            expect(r.state.turnPhase.kind).toBe("Interrupting");
            if (r.state.turnPhase.kind === "Interrupting") {
                expect(r.state.turnPhase.reason).toBe("user");
                expect(r.state.turnPhase.sigintSentAt).toBe(120);
            }
        });

        it("RequestStop while Idle is a same-ref no-op (PR G)", () => {
            // PR G removed the legacy `stopping` boolean. The pre-PR-G
            // behaviour flipped `stopping` true even with no turn in
            // flight; that was a latent bug surface (the stop could
            // not actually be acted on). The new behaviour leaves
            // state untouched and only emits the audit event.
            const start = mk();
            const r = update(start, { type: "RequestStop", at: 100 });
            expect(r.state).toBe(start);
            expect(r.state.turnPhase.kind).toBe("Idle");
            expect(r.events[0]).toMatchObject({ type: "stop-requested", at: 100 });
        });

        it("StopFailed while Interrupting rolls back to Streaming (subscribed)", () => {
            // Build up: ready → submit → re-subscribe to Streaming →
            // stop → stop-rpc-fails.
            const s0 = update(mk(), { type: "InitReady", at: 100 }).state;
            const s1 = update(s0, { type: "StreamSubscribe", at: 100 }).state;
            const s2 = update(s1, { type: "TurnStart", at: 110 }).state;
            const s3 = update(s2, { type: "StreamSubscribe", at: 120 }).state;
            expect(s3.turnPhase.kind).toBe("Streaming");
            const s4 = update(s3, { type: "RequestStop", at: 130 }).state;
            expect(s4.turnPhase.kind).toBe("Interrupting");
            const r = update(s4, { type: "StopFailed" });
            // Rolled back because stream is still subscribed.
            expect(r.state.turnPhase.kind).toBe("Streaming");
        });

        it("StopFailed when not Interrupting is a no-op (phase unchanged)", () => {
            // PR G: previously cleared the legacy `stopping` flag; now
            // there's nothing to clear and the phase is unchanged.
            const start = mk();
            const r = update(start, { type: "StopFailed" });
            expect(r.state.turnPhase.kind).toBe("Idle");
        });

        it("StreamUnsubscribe while working surfaces Disconnected.{lastKind, lastConnectedAt}", () => {
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            expect(s1.turnPhase.kind).toBe("Submitting");
            const r = update(s1, { type: "StreamUnsubscribe", at: 200 });
            // Subscription cleared (PR G: replaces `streaming.active=false`).
            expect(r.state.lastEventMs).toBeNull();
            expect(isWorking(r.state)).toBe(false);
            // Disconnected with Submitting as the lost kind,
            // lastConnectedAt = command.at, and a literal reason.
            expect(r.state.turnPhase.kind).toBe("Disconnected");
            if (r.state.turnPhase.kind === "Disconnected") {
                expect(r.state.turnPhase.lastKind).toBe("Submitting");
                expect(r.state.turnPhase.reason).toBe("stream-unsubscribed");
                expect(r.state.turnPhase.lastConnectedAt).toBe(200);
            }
        });

        it("StreamUnsubscribe while Idle is a same-ref no-op", () => {
            // An unsubscribe from a non-working phase (Idle / Done /
            // Disconnected) is idempotent and returns the same state
            // reference. The view does not observe a phantom
            // disconnect (no event emitted).
            const s0 = update(mk(), { type: "InitReady", at: 100 }).state;
            const s1 = update(s0, { type: "StreamSubscribe", at: 100 }).state;
            // Phase is still Idle (subscribe alone doesn't promote).
            expect(s1.turnPhase.kind).toBe("Idle");
            const r = update(s1, { type: "StreamUnsubscribe", at: 200 });
            // Same reference — no spurious tick to reactive consumers.
            expect(r.state).toBe(s1);
            expect(r.events).toEqual([]);
        });

        it("StreamFlushObserved while Submitting promotes phase to Streaming (codex P1 on #987)", () => {
            // Realistic runtime flow: subscribe ONCE at mount, then each
            // user message dispatches TurnStart. Submitting → Streaming
            // must promote on the first chunk arrival, NOT depend on a
            // re-subscribe that never fires in practice.
            const s0 = update(mk(), { type: "InitReady", at: 100 }).state;
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
            const s0 = update(mk(), { type: "InitReady", at: 100 }).state;
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
            const s0 = update(mk(), { type: "InitReady", at: 100 }).state;
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
            const s0 = update(mk(), { type: "InitReady", at: 100 }).state;
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
            const s0 = update(mk(), { type: "InitReady", at: 100 }).state;
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
            const s0 = update(mk(), { type: "InitReady", at: 100 }).state;
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
                type: "PendingMessageQueued", enqueuedWhileBusy: false,
                id: "m1",
                text: "hi",
                at: 105,
            }).state;
            expect(s1.turnPhase.kind).toBe("Idle");
            const s2 = update(s1, { type: "PendingMessageAccepted", id: "m1" }).state;
            expect(s2.turnPhase.kind).toBe("Idle");
            const s3 = update(s2, {
                type: "PendingMessageQueued", enqueuedWhileBusy: false,
                id: "m2",
                text: "hi2",
                at: 200,
            }).state;
            const s4 = update(s3, { type: "PendingMessageRejected", id: "m2" }).state;
            expect(s4.turnPhase.kind).toBe("Idle");
            const s5 = update(s4, {
                type: "PendingMessageQueued", enqueuedWhileBusy: false,
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
            const s1 = update(s0, { type: "InitReady", at: 100 }).state;
            expect(s1.turnPhase.kind).toBe("Idle");
            // s1 is already InitReady (terminal); InitFailed becomes a
            // no-op there. We just need to assert turnPhase stays Idle
            // either way — the orthogonality assertion still holds.
            const s2 = update(s1, { type: "InitFailed", at: 200, reason: "boom" }).state;
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

        it("isWorking tracks the working-phase set across a full turn", () => {
            // Walk a normal turn and assert isWorking flips at the
            // right boundaries. PR G: this replaced the "legacy ↔
            // phase dual-write invariant" test since the legacy
            // booleans no longer exist.
            const s0 = ready(100); // Idle
            expect(isWorking(s0)).toBe(false);

            const s1 = update(s0, { type: "TurnStart", at: 110 }).state; // Submitting
            expect(s1.turnPhase.kind).toBe("Submitting");
            expect(isWorking(s1)).toBe(true);

            const s2 = update(s1, { type: "StreamSubscribe", at: 120 }).state; // Streaming
            expect(s2.turnPhase.kind).toBe("Streaming");
            expect(isWorking(s2)).toBe(true);

            const s3 = update(s2, { type: "RequestStop", at: 130 }).state; // Interrupting
            expect(s3.turnPhase.kind).toBe("Interrupting");
            expect(isWorking(s3)).toBe(true);

            const s4 = update(s3, { type: "TurnEnd", stats: null }, 140).state; // Done
            expect(s4.turnPhase.kind).toBe("Done");
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
                    lastConnectedAt: 0,
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

    // ─────────────────────────────────────────────────────────────────
    // PR C — bounded `Interrupting → Done.interrupted`.
    //
    // Spec: docs/specs/SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md §8.
    // RequestStop while a turn is in flight schedules an interrupt
    // timeout (via the `schedule-interrupt-timeout` event); if no
    // graceful ack arrives, the dispatch layer fires
    // `InterruptTimeoutElapsed` and we force-transition to
    // Done.interrupted. The reducer guards against stale ticks by
    // checking the current phase on receipt.
    // ─────────────────────────────────────────────────────────────────
    describe("Bounded interrupt (PR C)", () => {
        it("RequestStop while Streaming emits schedule-interrupt-timeout", () => {
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, { type: "StreamSubscribe", at: 120 }).state;
            expect(s2.turnPhase.kind).toBe("Streaming");
            const r = update(s2, { type: "RequestStop", at: 200 });
            expect(r.state.turnPhase.kind).toBe("Interrupting");
            // Two events: stop-requested, and the timeout schedule.
            expect(r.events).toHaveLength(2);
            expect(r.events[0]).toMatchObject({
                type: "stop-requested",
                at: 200,
            });
            expect(r.events[1]).toMatchObject({
                type: "schedule-interrupt-timeout",
                deadlineMs: 200 + INTERRUPT_TIMEOUT_MS,
            });
        });

        it("RequestStop while Submitting also schedules the timeout", () => {
            // Phase enters Submitting before any chunk arrives. Stop
            // pressed here must still be bounded — same code path.
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            expect(s1.turnPhase.kind).toBe("Submitting");
            const r = update(s1, { type: "RequestStop", at: 150 });
            expect(r.state.turnPhase.kind).toBe("Interrupting");
            expect(r.events[1]).toMatchObject({
                type: "schedule-interrupt-timeout",
                deadlineMs: 150 + INTERRUPT_TIMEOUT_MS,
            });
        });

        it("RequestStop while already Interrupting does NOT re-schedule", () => {
            // A second Stop press must not double-arm the timeout
            // (spec §8: SIGINT only emitted once on entry, timeout
            // follows the same rule so the deadline isn't reset).
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, { type: "StreamSubscribe", at: 120 }).state;
            const s3 = update(s2, { type: "RequestStop", at: 200 }).state;
            expect(s3.turnPhase.kind).toBe("Interrupting");
            const r = update(s3, { type: "RequestStop", at: 250 });
            expect(r.state.turnPhase.kind).toBe("Interrupting");
            // Only the stop-requested event — no second schedule.
            expect(r.events).toHaveLength(1);
            expect(r.events[0]).toMatchObject({ type: "stop-requested" });
        });

        it("RequestStop while Idle does NOT schedule the timeout", () => {
            // Stop pressed without a turn in flight: legacy boolean
            // flips but no Interrupting phase, so no watchdog.
            const start = mk();
            const r = update(start, { type: "RequestStop", at: 100 });
            expect(r.state.turnPhase.kind).toBe("Idle");
            expect(r.events).toHaveLength(1);
            expect(r.events[0]).toMatchObject({ type: "stop-requested" });
        });

        it("RequestStop while Done does NOT schedule the timeout", () => {
            // After TurnEnd lands the phase in Done; a late Stop press
            // mustn't start a watchdog over an already-finished turn.
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, { type: "TurnEnd", stats: null }, 200).state;
            expect(s2.turnPhase.kind).toBe("Done");
            const r = update(s2, { type: "RequestStop", at: 250 });
            expect(r.events).toHaveLength(1);
            expect(r.events[0]).toMatchObject({ type: "stop-requested" });
        });

        it("InterruptTimeoutElapsed while Interrupting → Done.interrupted", () => {
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, { type: "StreamSubscribe", at: 120 }).state;
            const s3 = update(s2, { type: "RequestStop", at: 200 }).state;
            expect(s3.turnPhase.kind).toBe("Interrupting");
            const deadline = 200 + INTERRUPT_TIMEOUT_MS;
            const r = update(s3, {
                type: "InterruptTimeoutElapsed",
                at: deadline,
            });
            expect(r.state.turnPhase.kind).toBe("Done");
            if (r.state.turnPhase.kind === "Done") {
                expect(r.state.turnPhase.outcome).toBe("interrupted");
                expect(r.state.turnPhase.finishedAt).toBe(deadline);
            }
            // Per-turn sidecars cleared the same way TurnEnd does.
            expect(r.state.currentTool).toBe(null);
            expect(r.state.turnTokens).toBe(null);
            // Animation invariant: isWorking flips false.
            expect(isWorking(r.state)).toBe(false);
            // Audit event.
            expect(r.events).toHaveLength(1);
            expect(r.events[0]).toMatchObject({
                type: "interrupt-timed-out",
                at: deadline,
            });
        });

        it("InterruptTimeoutElapsed while Streaming is a no-op (agent acked first)", () => {
            // Realistic race: user pressed Stop, agent ack landed first
            // (TurnEnd → Done), then the user submitted again and we're
            // back in Streaming. The stale setTimeout finally fires —
            // it must not move us off Streaming.
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, { type: "StreamSubscribe", at: 120 }).state;
            expect(s2.turnPhase.kind).toBe("Streaming");
            const r = update(s2, {
                type: "InterruptTimeoutElapsed",
                at: 500,
            });
            // Reducer must return the SAME reference (no-op pattern).
            expect(r.state).toBe(s2);
            expect(r.events).toEqual([]);
        });

        it("InterruptTimeoutElapsed while Submitting is a no-op", () => {
            // Even more stale: phase rolled back to Submitting (e.g. a
            // fresh turn started before the timer fired).
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            expect(s1.turnPhase.kind).toBe("Submitting");
            const r = update(s1, {
                type: "InterruptTimeoutElapsed",
                at: 500,
            });
            expect(r.state).toBe(s1);
            expect(r.events).toEqual([]);
        });

        it("InterruptTimeoutElapsed while Idle is a no-op", () => {
            const start = mk();
            const r = update(start, {
                type: "InterruptTimeoutElapsed",
                at: 100,
            });
            expect(r.state).toBe(start);
            expect(r.events).toEqual([]);
        });

        it("InterruptTimeoutElapsed while Done is a no-op (graceful ack already landed)", () => {
            // The agent acked the SIGINT and we landed in Done.stopped
            // before the timer fired. The late tick must not overwrite
            // the outcome.
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, { type: "RequestStop", at: 200 }).state;
            expect(s2.turnPhase.kind).toBe("Interrupting");
            const s3 = update(s2, { type: "TurnEnd", stats: null }, 300).state;
            expect(s3.turnPhase.kind).toBe("Done");
            if (s3.turnPhase.kind === "Done") {
                expect(s3.turnPhase.outcome).toBe("stopped");
            }
            const r = update(s3, {
                type: "InterruptTimeoutElapsed",
                at: 500,
            });
            expect(r.state).toBe(s3);
            expect(r.events).toEqual([]);
            // Outcome preserved.
            if (r.state.turnPhase.kind === "Done") {
                expect(r.state.turnPhase.outcome).toBe("stopped");
            }
        });

        it("InterruptTimeoutElapsed while Disconnected is a no-op", () => {
            // Stream dropped after stop was requested → Disconnected.
            // The timeout that was armed must not corrupt that state.
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, { type: "RequestStop", at: 200 }).state;
            const s3 = update(s2, { type: "StreamUnsubscribe", at: 300 }).state;
            expect(s3.turnPhase.kind).toBe("Disconnected");
            const r = update(s3, {
                type: "InterruptTimeoutElapsed",
                at: 500,
            });
            expect(r.state).toBe(s3);
            expect(r.events).toEqual([]);
        });

        it("Three-paths invariant: TurnEnd, StreamUnsubscribe, and timeout all exit Interrupting", () => {
            // Spec §8: "Three independent paths into Done.Interrupted" —
            // the working animation can never get stuck.
            const buildInterrupting = () => {
                const a = ready(100);
                const b = update(a, { type: "TurnStart", at: 110 }).state;
                const c = update(b, { type: "StreamSubscribe", at: 120 }).state;
                return update(c, { type: "RequestStop", at: 200 }).state;
            };

            // Path 1: graceful TurnEnd.
            const p1 = update(buildInterrupting(), {
                type: "TurnEnd",
                stats: null,
            }, 300);
            expect(isWorking(p1.state)).toBe(false);
            expect(p1.state.turnPhase.kind).toBe("Done");

            // Path 2: stream torn down → Disconnected (also not working).
            const p2 = update(buildInterrupting(), {
                type: "StreamUnsubscribe",
                at: 300,
            });
            expect(isWorking(p2.state)).toBe(false);
            expect(p2.state.turnPhase.kind).toBe("Disconnected");

            // Path 3: bounded timeout.
            const p3 = update(buildInterrupting(), {
                type: "InterruptTimeoutElapsed",
                at: 200 + INTERRUPT_TIMEOUT_MS,
            });
            expect(isWorking(p3.state)).toBe(false);
            expect(p3.state.turnPhase.kind).toBe("Done");
            if (p3.state.turnPhase.kind === "Done") {
                expect(p3.state.turnPhase.outcome).toBe("interrupted");
            }
        });

        it("INTERRUPT_TIMEOUT_MS is 5000 (sanity)", () => {
            // Pinned so a sneaky bump can't slip in unreviewed.
            expect(INTERRUPT_TIMEOUT_MS).toBe(5_000);
        });
    });

    // ─────────────────────────────────────────────────────────────────
    // PR D — bounded `Submitting → Done.errored`.
    //
    // Spec: docs/specs/SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md §8 /
    // issue #728 gap 2. TurnStart promotes the phase to Submitting and
    // emits `schedule-submit-timeout`; if no stream activity arrives
    // (no flush, no tool, no token) within SUBMIT_TIMEOUT_MS, the
    // dispatch layer fires `SubmitTimeoutElapsed` and we force-transition
    // to Done.errored. The reducer guards against stale ticks by
    // checking the current phase on receipt.
    // ─────────────────────────────────────────────────────────────────
    describe("Bounded submit timeout (PR D)", () => {
        it("TurnStart emits schedule-submit-timeout with correct deadlineMs", () => {
            const s0 = ready(100);
            const r = update(s0, { type: "TurnStart", at: 110 });
            expect(r.state.turnPhase.kind).toBe("Submitting");
            // Two events: turn-started, and the submit-timeout schedule.
            expect(r.events).toHaveLength(2);
            expect(r.events[0]).toMatchObject({
                type: "turn-started",
                at: 110,
            });
            expect(r.events[1]).toMatchObject({
                type: "schedule-submit-timeout",
                deadlineMs: 110 + SUBMIT_TIMEOUT_MS,
            });
        });

        it("Suppressed TurnStart does NOT emit schedule-submit-timeout", () => {
            // Stream inactive → suppressed; only the suppression event,
            // no watchdog arming.
            const start = mk();
            const r = update(start, { type: "TurnStart", at: 100 });
            expect(r.events).toHaveLength(1);
            expect(r.events[0]).toMatchObject({
                type: "turn-start-suppressed",
            });
        });

        it("TurnStart while still loading history does NOT emit schedule-submit-timeout", () => {
            // Subscribed but not InitReady — invariant 2 suppresses;
            // watchdog must not arm against a turn that never started.
            const s0 = update(mk(), { type: "StreamSubscribe", at: 100 }).state;
            const r = update(s0, { type: "TurnStart", at: 110 });
            expect(r.events).toHaveLength(1);
            expect(r.events[0]).toMatchObject({
                type: "turn-start-suppressed",
                reason: "init still loading",
            });
        });

        it("SubmitTimeoutElapsed while Submitting → Done.errored", () => {
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            expect(s1.turnPhase.kind).toBe("Submitting");
            const deadline = 110 + SUBMIT_TIMEOUT_MS;
            const r = update(s1, {
                type: "SubmitTimeoutElapsed",
                at: deadline,
            });
            expect(r.state.turnPhase.kind).toBe("Done");
            if (r.state.turnPhase.kind === "Done") {
                expect(r.state.turnPhase.outcome).toBe("errored");
                expect(r.state.turnPhase.finishedAt).toBe(deadline);
            }
            // Per-turn sidecars cleared the same way TurnEnd does.
            expect(r.state.currentTool).toBe(null);
            expect(r.state.turnTokens).toBe(null);
            // Animation invariant: isWorking flips false.
            expect(isWorking(r.state)).toBe(false);
            // Audit event.
            expect(r.events).toHaveLength(1);
            expect(r.events[0]).toMatchObject({
                type: "submit-timed-out",
                at: deadline,
            });
        });

        it("SubmitTimeoutElapsed clears live tool + tokens that snuck in pre-promotion", () => {
            // Edge case: the dispatch layer might set currentTool /
            // turnTokens during Submitting before the phase promotion
            // path actually runs (e.g. an ill-ordered call site). The
            // forced-Done.errored path must still scrub the sidecars so
            // the next turn starts clean.
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            // Inject live per-turn sidecars without dispatching the
            // commands (which would also promote to Streaming via
            // bumpEvent and defeat the test).
            const sDirty: AgentPaneState = {
                ...s1,
                currentTool: "Read",
                turnTokens: { input: 42, output: 17 },
            };
            const r = update(sDirty, {
                type: "SubmitTimeoutElapsed",
                at: 5_000,
            });
            expect(r.state.turnPhase.kind).toBe("Done");
            expect(r.state.currentTool).toBe(null);
            expect(r.state.turnTokens).toBe(null);
        });

        it("SubmitTimeoutElapsed while Streaming is a no-op (same ref)", () => {
            // Realistic race: the first chunk arrived (Submitting →
            // Streaming via StreamFlushObserved) just before the timer
            // fired. The reducer must not corrupt the live Streaming
            // phase.
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, {
                type: "StreamFlushObserved",
                addedCount: 1,
                at: 120,
            }).state;
            expect(s2.turnPhase.kind).toBe("Streaming");
            const r = update(s2, {
                type: "SubmitTimeoutElapsed",
                at: 30_110,
            });
            expect(r.state).toBe(s2);
            expect(r.events).toEqual([]);
        });

        it("SubmitTimeoutElapsed while Interrupting is a no-op (same ref)", () => {
            // User pressed Stop while still in Submitting (e.g. instant
            // regret). Phase moved to Interrupting; submit watchdog
            // should not retroactively classify as errored.
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, { type: "RequestStop", at: 120 }).state;
            expect(s2.turnPhase.kind).toBe("Interrupting");
            const r = update(s2, {
                type: "SubmitTimeoutElapsed",
                at: 30_110,
            });
            expect(r.state).toBe(s2);
            expect(r.events).toEqual([]);
        });

        it("SubmitTimeoutElapsed while Idle is a no-op (same ref)", () => {
            const start = mk();
            const r = update(start, {
                type: "SubmitTimeoutElapsed",
                at: 100,
            });
            expect(r.state).toBe(start);
            expect(r.events).toEqual([]);
        });

        it("SubmitTimeoutElapsed while Done is a no-op (same ref)", () => {
            // Graceful TurnEnd already landed → Done.completed.
            // Late submit watchdog must not overwrite the outcome.
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, { type: "TurnEnd", stats: null }, 200).state;
            expect(s2.turnPhase.kind).toBe("Done");
            if (s2.turnPhase.kind === "Done") {
                expect(s2.turnPhase.outcome).toBe("completed");
            }
            const r = update(s2, {
                type: "SubmitTimeoutElapsed",
                at: 30_110,
            });
            expect(r.state).toBe(s2);
            expect(r.events).toEqual([]);
            // Outcome preserved.
            if (r.state.turnPhase.kind === "Done") {
                expect(r.state.turnPhase.outcome).toBe("completed");
            }
        });

        it("SubmitTimeoutElapsed while Disconnected is a no-op (same ref)", () => {
            // Stream dropped mid-Submitting → Disconnected.lastKind=
            // Submitting. The submit watchdog must not corrupt that.
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            expect(s1.turnPhase.kind).toBe("Submitting");
            const s2 = update(s1, { type: "StreamUnsubscribe", at: 200 }).state;
            expect(s2.turnPhase.kind).toBe("Disconnected");
            const r = update(s2, {
                type: "SubmitTimeoutElapsed",
                at: 30_110,
            });
            expect(r.state).toBe(s2);
            expect(r.events).toEqual([]);
        });

        it("Three-paths invariant: bumpEvent, StreamFlushObserved, and timeout all exit Submitting", () => {
            // Spec §8 — "Three independent paths out of Submitting" — so
            // the working animation can never hang on Submitting.
            const buildSubmitting = () => {
                const a = ready(100);
                return update(a, { type: "TurnStart", at: 110 }).state;
            };

            // Path 1: promotion via bumpEvent (a tool starts).
            const p1 = update(
                buildSubmitting(),
                { type: "ToolStart", name: "Read" },
                130,
            );
            expect(p1.state.turnPhase.kind).toBe("Streaming");

            // Path 1b: promotion via bumpEvent (token activity).
            const p1b = update(
                buildSubmitting(),
                { type: "TokensIn", input: 10 },
                130,
            );
            expect(p1b.state.turnPhase.kind).toBe("Streaming");

            // Path 2: promotion via StreamFlushObserved.
            const p2 = update(buildSubmitting(), {
                type: "StreamFlushObserved",
                addedCount: 1,
                at: 130,
            });
            expect(p2.state.turnPhase.kind).toBe("Streaming");

            // Path 3: bounded timeout — no stream activity arrived.
            const p3 = update(buildSubmitting(), {
                type: "SubmitTimeoutElapsed",
                at: 110 + SUBMIT_TIMEOUT_MS,
            });
            expect(isWorking(p3.state)).toBe(false);
            expect(p3.state.turnPhase.kind).toBe("Done");
            if (p3.state.turnPhase.kind === "Done") {
                expect(p3.state.turnPhase.outcome).toBe("errored");
            }
        });

        it("Late TurnEnd after SubmitTimeoutElapsed does NOT overwrite errored (first-done-wins)", () => {
            // PR C added `alreadyDone` to the TurnEnd arm so a late
            // graceful ack after a forced Done can't reclassify the
            // outcome. PR D inherits the same guarantee: timeout fires,
            // turn lands in Done.errored, a stray backend TurnEnd that
            // arrives afterwards must preserve "errored", not overwrite
            // with "completed" or "stopped".
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const deadline = 110 + SUBMIT_TIMEOUT_MS;
            const s2 = update(s1, {
                type: "SubmitTimeoutElapsed",
                at: deadline,
            }).state;
            expect(s2.turnPhase.kind).toBe("Done");
            if (s2.turnPhase.kind === "Done") {
                expect(s2.turnPhase.outcome).toBe("errored");
            }
            // Late backend ack — graceful TurnEnd lands.
            const r = update(s2, { type: "TurnEnd", stats: null }, deadline + 1_000);
            expect(r.state.turnPhase.kind).toBe("Done");
            if (r.state.turnPhase.kind === "Done") {
                // First-done-wins: outcome stays "errored".
                expect(r.state.turnPhase.outcome).toBe("errored");
                // finishedAt is also pinned (not bumped to the late ack).
                expect(r.state.turnPhase.finishedAt).toBe(deadline);
            }
        });

        it("SUBMIT_TIMEOUT_MS is 30000 (sanity)", () => {
            // Pinned so a sneaky bump can't slip in unreviewed.
            expect(SUBMIT_TIMEOUT_MS).toBe(30_000);
        });
    });

    // ─────────────────────────────────────────────────────────────────
    // Stream resume after a reconnect (PR F / #1523).
    //
    // A stream drop + resubscribe lands the phase in Idle. Resumed LIVE
    // content (flush / tool / token) must re-enter Streaming so the
    // "in progress" indicator reflects ongoing output.
    //
    // NOTE: the bounded `StreamStalled` streaming-idle watchdog that used
    // to live here was dead code — the reducer emitted
    // `schedule-stream-watchdog` but no dispatcher ever consumed it, so
    // `StreamStalled` never fired in production. It was removed; see
    // docs/analysis/ANALYSIS_DEAD_STREAM_STALLED_WATCHDOG_2026_06_18.md.
    // ─────────────────────────────────────────────────────────────────
    describe("Stream resume after reconnect (re-enter Streaming)", () => {

        it("StreamFlushObserved from Idle after a reconnect promotes to Streaming (resumed content ⇒ working)", () => {
            // A stream drop + resubscribe lands the phase in Idle (PR F). Resumed
            // LIVE content must re-enter the working set, or the "in progress"
            // indicator stays off while output streams (the kill+respawn-during-
            // stall bug). See ANALYSIS_AGENT_WORKING_STATE_AND_INPUT_REDUCER.
            const s0 = update(mk(), { type: "InitReady", at: 100 }).state;
            const s1 = update(s0, { type: "StreamSubscribe", at: 100 }).state;
            const s2 = update(s1, { type: "TurnStart", at: 110 }).state;
            const s3 = update(s2, { type: "StreamFlushObserved", addedCount: 1, at: 120 }).state;
            expect(s3.turnPhase.kind).toBe("Streaming");
            const s4 = update(s3, { type: "StreamUnsubscribe", at: 130 }).state;
            expect(s4.turnPhase.kind).toBe("Disconnected");
            const s5 = update(s4, { type: "StreamSubscribe", at: 140 }).state;
            expect(s5.turnPhase.kind).toBe("Idle"); // PR F: reconnect → Idle
            const r = update(s5, { type: "StreamFlushObserved", addedCount: 2, at: 150 });
            expect(r.state.turnPhase.kind).toBe("Streaming");
        });

        it("ToolStart/TokensIn from Idle after a reconnect promotes to Streaming", () => {
            const base = (() => {
                const s0 = update(mk(), { type: "InitReady", at: 100 }).state;
                const s1 = update(s0, { type: "StreamSubscribe", at: 100 }).state;
                const s2 = update(s1, { type: "TurnStart", at: 110 }).state;
                const s3 = update(s2, { type: "StreamFlushObserved", addedCount: 1, at: 120 }).state;
                const s4 = update(s3, { type: "StreamUnsubscribe", at: 130 }).state;
                const s5 = update(s4, { type: "StreamSubscribe", at: 140 }).state;
                expect(s5.turnPhase.kind).toBe("Idle");
                return s5;
            })();
            expect(update(base, { type: "ToolStart", name: "Bash" }, 150).state.turnPhase.kind).toBe("Streaming");
            expect(update(base, { type: "TokensIn", input: 10 }, 150).state.turnPhase.kind).toBe("Streaming");
        });

    });

    // ─────────────────────────────────────────────────────────────────
    // PR F — `Disconnected` phase + banner contract.
    //
    // Spec: docs/specs/SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md §6.4.
    // StreamUnsubscribe from a working phase (Submitting / Streaming /
    // Interrupting) transitions to Disconnected — carrying `lastKind`,
    // `lastConnectedAt`, and a finite literal `reason`. Non-working
    // phases (Idle / Done / Disconnected) make StreamUnsubscribe a
    // same-ref no-op. StreamSubscribe from Disconnected resets to
    // Idle. A late TurnEnd while Disconnected is preserved (the
    // disconnect is the authoritative outcome).
    // ─────────────────────────────────────────────────────────────────
    describe("Disconnected phase (PR F)", () => {
        it("StreamUnsubscribe while Streaming → Disconnected; legacy cleared; isWorking=false; emits stream-disconnected", () => {
            // Reach Streaming via the realistic path (subscribe once,
            // then TurnStart, then a flush promotes).
            const s0 = update(mk(), { type: "InitReady", at: 100 }).state;
            const s1 = update(s0, { type: "StreamSubscribe", at: 100 }).state;
            const s2 = update(s1, { type: "TurnStart", at: 110 }).state;
            const s3 = update(s2, {
                type: "StreamFlushObserved",
                addedCount: 1,
                at: 200,
            }).state;
            expect(s3.turnPhase.kind).toBe("Streaming");
            // Tool + tokens so we can verify they're cleared on
            // disconnect (cleanup mirrors the bounded force-transition
            // arms — see reducer.ts §StreamUnsubscribe).
            const s4 = update(s3, { type: "ToolStart", name: "Read" }, 210)
                .state;
            const s5 = update(s4, { type: "TokensIn", input: 42 }, 220)
                .state;
            expect(s5.currentTool).toBe("Read");
            expect(s5.turnTokens?.input).toBe(42);

            const r = update(s5, { type: "StreamUnsubscribe", at: 300 });
            // Phase transitions to Disconnected with lastKind = Streaming.
            expect(r.state.turnPhase.kind).toBe("Disconnected");
            if (r.state.turnPhase.kind === "Disconnected") {
                expect(r.state.turnPhase.lastKind).toBe("Streaming");
                expect(r.state.turnPhase.lastConnectedAt).toBe(300);
                expect(r.state.turnPhase.reason).toBe("stream-unsubscribed");
            }
            // Subscription cleared (PR G: `lastEventMs !== null` replaces
            // the legacy `streaming.active` boolean).
            expect(r.state.lastEventMs).toBeNull();
            // Per-turn sidecars cleared.
            expect(r.state.currentTool).toBe(null);
            expect(r.state.turnTokens).toBe(null);
            // Animation invariant: isWorking false; isDisconnected true.
            expect(isWorking(r.state)).toBe(false);
            expect(isDisconnected(r.state)).toBe(true);
            // Two events: stream-unsubscribed (always) + stream-disconnected.
            expect(r.events).toHaveLength(2);
            expect(r.events[0]).toMatchObject({
                type: "stream-unsubscribed",
                at: 300,
            });
            expect(r.events[1]).toMatchObject({
                type: "stream-disconnected",
                at: 300,
                lastKind: "Streaming",
                reason: "stream-unsubscribed",
            });
        });

        it("StreamUnsubscribe while Submitting → Disconnected.{lastKind=Submitting}", () => {
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            expect(s1.turnPhase.kind).toBe("Submitting");
            const r = update(s1, { type: "StreamUnsubscribe", at: 200 });
            expect(r.state.turnPhase.kind).toBe("Disconnected");
            if (r.state.turnPhase.kind === "Disconnected") {
                expect(r.state.turnPhase.lastKind).toBe("Submitting");
                expect(r.state.turnPhase.lastConnectedAt).toBe(200);
                expect(r.state.turnPhase.reason).toBe("stream-unsubscribed");
            }
            expect(isDisconnected(r.state)).toBe(true);
            // stream-disconnected event surfaces the lost kind.
            expect(r.events[1]).toMatchObject({
                type: "stream-disconnected",
                lastKind: "Submitting",
            });
        });

        it("StreamUnsubscribe while Interrupting → Disconnected.{lastKind=Interrupting}", () => {
            // Reach Interrupting via the standard path.
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, { type: "StreamSubscribe", at: 120 }).state;
            const s3 = update(s2, { type: "RequestStop", at: 200 }).state;
            expect(s3.turnPhase.kind).toBe("Interrupting");
            const r = update(s3, { type: "StreamUnsubscribe", at: 300 });
            expect(r.state.turnPhase.kind).toBe("Disconnected");
            if (r.state.turnPhase.kind === "Disconnected") {
                expect(r.state.turnPhase.lastKind).toBe("Interrupting");
                expect(r.state.turnPhase.lastConnectedAt).toBe(300);
            }
            // Working phase exited (PR G: was previously asserted on the
            // legacy `stopping` boolean).
            expect(isWorking(r.state)).toBe(false);
            expect(isDisconnected(r.state)).toBe(true);
        });

        it("StreamUnsubscribe while Idle is a same-ref no-op", () => {
            // Already in Idle (no working state to disconnect from).
            // The unsubscribe is non-news; no event, no state churn.
            const s0 = ready(100); // Idle + subscribed (lastEventMs set)
            expect(s0.turnPhase.kind).toBe("Idle");
            const r = update(s0, { type: "StreamUnsubscribe", at: 200 });
            expect(r.state).toBe(s0);
            expect(r.events).toEqual([]);
        });

        it("StreamUnsubscribe while Done is a same-ref no-op", () => {
            // Graceful TurnEnd landed → Done. A later teardown
            // (component unmount, etc.) must not corrupt the outcome.
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, { type: "TurnEnd", stats: null }, 200).state;
            expect(s2.turnPhase.kind).toBe("Done");
            const r = update(s2, { type: "StreamUnsubscribe", at: 300 });
            expect(r.state).toBe(s2);
            expect(r.events).toEqual([]);
            // Outcome preserved.
            if (r.state.turnPhase.kind === "Done") {
                expect(r.state.turnPhase.outcome).toBe("completed");
            }
        });

        it("StreamUnsubscribe while Disconnected is a same-ref no-op", () => {
            // Idempotent: a duplicate unsubscribe (e.g. socket teardown
            // fires twice during cleanup) must not synthesise a fresh
            // Disconnected with a newer `lastConnectedAt` — the original
            // disconnect timestamp is the authoritative one.
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, { type: "StreamUnsubscribe", at: 200 })
                .state;
            expect(s2.turnPhase.kind).toBe("Disconnected");
            const r = update(s2, { type: "StreamUnsubscribe", at: 300 });
            expect(r.state).toBe(s2);
            expect(r.events).toEqual([]);
            // lastConnectedAt pinned to the original disconnect.
            if (r.state.turnPhase.kind === "Disconnected") {
                expect(r.state.turnPhase.lastConnectedAt).toBe(200);
            }
        });

        it("StreamSubscribe from Disconnected → Idle (does NOT auto-resume Streaming)", () => {
            // Spec §6.4: a fresh subscribe after disconnect resets the
            // working state. The lost turn is gone; the user must press
            // send again to start a new one (next TurnStart promotes
            // back to Submitting).
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, {
                type: "StreamFlushObserved",
                addedCount: 1,
                at: 200,
            }).state;
            expect(s2.turnPhase.kind).toBe("Streaming");
            const s3 = update(s2, { type: "StreamUnsubscribe", at: 300 })
                .state;
            expect(s3.turnPhase.kind).toBe("Disconnected");
            // Now the dispatcher (or the reconnect button) re-subscribes.
            const r = update(s3, { type: "StreamSubscribe", at: 400 });
            expect(r.state.turnPhase.kind).toBe("Idle");
            // Subscribed again (PR G: `lastEventMs !== null` replaces
            // the legacy `streaming.active === true` check) so a fresh
            // TurnStart can fire.
            expect(r.state.lastEventMs).toBe(400);
            // No working state → no watchdog re-arm event. Just the
            // standard `stream-subscribed` event.
            expect(r.events).toHaveLength(1);
            expect(r.events[0]).toMatchObject({
                type: "stream-subscribed",
                at: 400,
            });
        });

        it("Late TurnEnd while Disconnected preserves the disconnect (Option A; first-done-wins)", () => {
            // PR F spec §4 Option A: the disconnect IS the outcome.
            // A late TurnEnd from the backend (e.g. a final session_end
            // that was buffered just before the PTY died, then drained
            // post-mortem) must NOT overwrite Disconnected with Done.
            const s0 = ready(100);
            const s1 = update(s0, { type: "TurnStart", at: 110 }).state;
            const s2 = update(s1, {
                type: "StreamFlushObserved",
                addedCount: 1,
                at: 200,
            }).state;
            const s3 = update(s2, { type: "StreamUnsubscribe", at: 300 })
                .state;
            expect(s3.turnPhase.kind).toBe("Disconnected");
            // Late ack arrives.
            const r = update(
                s3,
                {
                    type: "TurnEnd",
                    stats: { input_tokens: 5, output_tokens: 10 } as any,
                },
                400,
            );
            // Same reference — Disconnected preserved entirely.
            expect(r.state).toBe(s3);
            expect(r.events).toEqual([]);
            // Phase + payload intact.
            expect(r.state.turnPhase.kind).toBe("Disconnected");
            if (r.state.turnPhase.kind === "Disconnected") {
                expect(r.state.turnPhase.lastConnectedAt).toBe(300);
                expect(r.state.turnPhase.lastKind).toBe("Streaming");
            }
        });

        it("isDisconnected selector matches the phase kind", () => {
            // Cross-product sanity — `isDisconnected(state)` is a pure
            // projection of `turnPhase.kind === "Disconnected"`. Wired
            // separately from `isWorking` so a single mistaken edit can
            // be caught.
            const idle = ready(100);
            expect(isDisconnected(idle)).toBe(false);
            const submitting = update(idle, { type: "TurnStart", at: 110 })
                .state;
            expect(isDisconnected(submitting)).toBe(false);
            const disc = update(submitting, {
                type: "StreamUnsubscribe",
                at: 200,
            }).state;
            expect(isDisconnected(disc)).toBe(true);
            const done = update(idle, { type: "TurnEnd", stats: null }, 300)
                .state;
            // (Done from Idle path: TurnEnd from Idle promotes to Done
            // via the same arm — see existing tests.)
            expect(isDisconnected(done)).toBe(false);
        });
    });

    // ── Composer details / Log panel ────────────────────────────────
    describe("Composer details panel", () => {
        it("initial state: detailsOpen false", () => {
            const s = mk();
            expect(s.detailsOpen).toBe(false);
        });

        it("DetailsToggle: closed → open", () => {
            let s = mk();
            const r = update(s, { type: "DetailsToggle" });
            expect(r.state.detailsOpen).toBe(true);
            expect(r.events).toEqual([]);
        });

        it("DetailsToggle: open → closed", () => {
            let s = mk();
            s = update(s, { type: "DetailsToggle" }).state;
            s = update(s, { type: "DetailsToggle" }).state;
            expect(s.detailsOpen).toBe(false);
        });

        it("DetailsExpand: idempotent when already open", () => {
            let s = mk();
            s = update(s, { type: "DetailsExpand" }).state;
            const r = update(s, { type: "DetailsExpand" });
            expect(r.state).toBe(s);
        });

        it("DetailsExpand: opens when closed", () => {
            const s = mk();
            const r = update(s, { type: "DetailsExpand" });
            expect(r.state.detailsOpen).toBe(true);
        });

        it("DetailsCollapse: idempotent when already closed", () => {
            const s = mk();
            const r = update(s, { type: "DetailsCollapse" });
            expect(r.state).toBe(s);
        });

        it("DetailsCollapse: closes an open panel", () => {
            let s = mk();
            s = update(s, { type: "DetailsExpand" }).state;
            const r = update(s, { type: "DetailsCollapse" });
            expect(r.state.detailsOpen).toBe(false);
        });

        it("TurnStart auto-collapses an open details panel", () => {
            // Cross-arm interaction: sending a message hides the panel
            // so the user isn't dropdown-staring as the turn begins.
            let s = ready(100);
            s = update(s, { type: "DetailsExpand" }).state;
            expect(s.detailsOpen).toBe(true);
            const r = update(s, { type: "TurnStart", at: 200 });
            expect(r.state.detailsOpen).toBe(false);
            expect(r.state.turnPhase.kind).toBe("Submitting");
        });
    });
});

// `workingByLegacy` was the dual-write invariant helper (turnActive ||
// stopping) — removed in PR G alongside the legacy fields it read.
// Use `isWorking(state)` directly.
