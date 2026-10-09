// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The pane reducer: stop, pending messages, init and watchdog.

import { describe, expect, it } from "vitest";
import { update } from "./reducer";
import { isInitReady, isWorking, LIVENESS_RECOVERY_MS, STUCK_THRESHOLD_MS, TurnPhase } from "./types";
import { streaming, mk, ready } from "./reducer-test-fixtures";

describe("agent-pane-state reducer", () => {
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

        // SPEC_AGENT_WORKING_STATE_UNIFICATION_2026_09_04.md Phase 1
        // (codex P2 on PR #2970, corrected design): `flushing` is set at the
        // moment flushHeldMessages actually starts delivering a held
        // message — NOT unconditionally on TurnEnd, which can be well
        // before delivery genuinely begins if a controller refresh is still
        // deferred. This closes the "Worked" + stale "Queued — sends at
        // next step" copy desync without ever claiming "Sending" for a
        // message that hasn't actually started sending.
        it("PendingMessageFlushStarted marks the matching entry flushing", () => {
            const s0 = update(mk(), {
                type: "PendingMessageQueued",
                enqueuedWhileBusy: true,
                id: "m1",
                text: "hi",
                at: 150,
            }).state;
            expect(s0.pending[0].flushing).toBeFalsy();
            const r = update(s0, { type: "PendingMessageFlushStarted", id: "m1" });
            expect(r.state.pending).toHaveLength(1);
            expect(r.state.pending[0]).toMatchObject({ id: "m1", flushing: true });
            expect(r.events[0]).toMatchObject({ type: "pending-flush-started", id: "m1" });
        });

        it("PendingMessageFlushStarted only touches the matching id, leaving others untouched", () => {
            const s0 = update(
                update(mk(), {
                    type: "PendingMessageQueued", enqueuedWhileBusy: true, id: "a", text: "1", at: 100,
                }).state,
                { type: "PendingMessageQueued", enqueuedWhileBusy: true, id: "b", text: "2", at: 110 },
            ).state;
            const r = update(s0, { type: "PendingMessageFlushStarted", id: "a" });
            expect(r.state.pending.find((m) => m.id === "a")?.flushing).toBe(true);
            expect(r.state.pending.find((m) => m.id === "b")?.flushing).toBeFalsy();
        });

        it("PendingMessageFlushStarted for an unknown id is a same-ref no-op", () => {
            const start = mk();
            const r = update(start, { type: "PendingMessageFlushStarted", id: "ghost" });
            expect(r.state).toBe(start);
            expect(r.events).toEqual([]);
        });

        it("PendingMessageFlushStarted is idempotent on an already-flushing entry (no redundant array churn)", () => {
            const s0 = update(mk(), {
                type: "PendingMessageQueued",
                enqueuedWhileBusy: true,
                id: "m1",
                text: "hi",
                at: 150,
            }).state;
            const s1 = update(s0, { type: "PendingMessageFlushStarted", id: "m1" }).state;
            const r2 = update(s1, { type: "PendingMessageFlushStarted", id: "m1" });
            expect(r2.state).toBe(s1);
            expect(r2.events).toEqual([]);
        });

        it("TurnEnd does NOT mark pending entries flushing — that's PendingMessageFlushStarted's job now", () => {
            const s0 = update(streaming(100), {
                type: "PendingMessageQueued",
                enqueuedWhileBusy: true,
                id: "m1",
                text: "hi",
                at: 150,
            }).state;
            const r = update(s0, { type: "TurnEnd", stats: null }, 200);
            expect(r.state.turnPhase.kind).toBe("Done");
            expect(r.state.pending[0].flushing).toBeFalsy();
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
            // Below LIVENESS_RECOVERY_MS the watchdog only diagnoses — no mutation.
            expect(r.state).toBe(s0);
        });

        it("StreamWatchdogTick past LIVENESS_RECOVERY_MS recovers a hung Streaming turn to Idle", () => {
            const s0 = streaming(1_000); // Streaming, toolsActive 0, lastEventMs 1000
            expect(isWorking(s0)).toBe(true);
            const tick = 1_000 + LIVENESS_RECOVERY_MS + 1_000;
            const r = update(s0, { type: "StreamWatchdogTick", nowMs: tick });
            expect(r.state.turnPhase.kind).toBe("Idle");
            expect(isWorking(r.state)).toBe(false);
            expect(r.state.currentTool).toBeNull();
            expect(r.state.turnTokens).toBeNull();
            expect(r.events[0]).toMatchObject({
                type: "working-recovered",
                thresholdMs: LIVENESS_RECOVERY_MS,
            });
        });

        it("StreamWatchdogTick does NOT recover while a tool is active (emits stream-stuck)", () => {
            const base = streaming(1_000);
            // A running tool keeps the turn alive; lastEventMs bumped to 2000.
            const s0 = update(base, { type: "ToolStart", name: "Bash" }, 2_000).state;
            expect((s0.turnPhase as Extract<TurnPhase, { kind: "Streaming" }>).toolsActive).toBe(1);
            const tick = 2_000 + LIVENESS_RECOVERY_MS + 5_000;
            const r = update(s0, { type: "StreamWatchdogTick", nowMs: tick });
            expect(r.state.turnPhase.kind).toBe("Streaming");
            expect(r.events[0]).toMatchObject({ type: "stream-stuck" });
        });

        it("StreamWatchdogTick does NOT recover a rate-limited turn within retryAfterMs + LIVENESS window", () => {
            // A genuine 429 backoff re-emits provider_waiting within retryAfterMs,
            // so the recovery threshold is retryAfterMs + LIVENESS_RECOVERY_MS. A
            // tick past LIVENESS alone (but short of the sum) must NOT recover.
            const base = streaming(1_000);
            const s0 = update(base, {
                type: "ProviderWaiting",
                reason: "rate_limited",
                retryAfterMs: 30_000,
                at: 2_000,
            }).state;
            const phase = s0.turnPhase as Extract<TurnPhase, { kind: "Streaming" }>;
            expect(phase.waitingReason).toBe("rate_limited");
            // idle = LIVENESS + 5s, still < retryAfterMs(30s) + LIVENESS.
            const tick = (s0.lastEventMs ?? 2_000) + LIVENESS_RECOVERY_MS + 5_000;
            const r = update(s0, { type: "StreamWatchdogTick", nowMs: tick });
            expect(r.state.turnPhase.kind).toBe("Streaming");
            expect(r.events[0]).toMatchObject({ type: "stream-stuck" });
        });

        it("StreamWatchdogTick recovers a stalled rate-limited turn past retryAfterMs + LIVENESS window", () => {
            const base = streaming(1_000);
            const retryAfterMs = 30_000;
            const s0 = update(base, {
                type: "ProviderWaiting",
                reason: "rate_limited",
                retryAfterMs,
                at: 2_000,
            }).state;
            // idle past retryAfterMs + LIVENESS → the retry loop stalled (no
            // follow-up provider_waiting / token / session_end); recover to Idle.
            const tick = (s0.lastEventMs ?? 2_000) + retryAfterMs + LIVENESS_RECOVERY_MS + 1_000;
            const r = update(s0, { type: "StreamWatchdogTick", nowMs: tick });
            expect(r.state.turnPhase.kind).toBe("Idle");
            expect(isWorking(r.state)).toBe(false);
            expect(r.events[0]).toMatchObject({
                type: "working-recovered",
                thresholdMs: retryAfterMs + LIVENESS_RECOVERY_MS,
            });
        });

        it("StreamWatchdogTick recovers a stalled rate-limited turn with null retryAfterMs at the LIVENESS window", () => {
            const base = streaming(1_000);
            const s0 = update(base, {
                type: "ProviderWaiting",
                reason: "rate_limited",
                retryAfterMs: null,
                at: 2_000,
            }).state;
            // null retryAfterMs → threshold falls back to LIVENESS_RECOVERY_MS.
            const tick = (s0.lastEventMs ?? 2_000) + LIVENESS_RECOVERY_MS + 1_000;
            const r = update(s0, { type: "StreamWatchdogTick", nowMs: tick });
            expect(r.state.turnPhase.kind).toBe("Idle");
            expect(r.events[0]).toMatchObject({
                type: "working-recovered",
                thresholdMs: LIVENESS_RECOVERY_MS,
            });
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

});
