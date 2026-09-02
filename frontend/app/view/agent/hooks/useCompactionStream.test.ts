// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { TurnPhase } from "@/app/store/agent-pane-state/types";

const hub = vi.hoisted(() => ({
    handlers: new Map<string, (e: unknown) => void>(),
}));

vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: vi.fn((sub: { eventType: string; handler: (e: unknown) => void }) => {
        hub.handlers.set(sub.eventType, sub.handler);
        return () => hub.handlers.delete(sub.eventType);
    }),
}));

import {
    MISSED_PING_RETRY_WINDOW_MS,
    resolveCompactionStart,
    useCompactionStream,
    wasCompactionStartedAccepted,
} from "./useCompactionStream";

// Codex P1 on PR #2378: `compaction_started` is a persisted WPS event
// (persist: 1) with no completion tombstone — a late/reconnecting
// subscriber replays it verbatim even long after the matching
// compaction finished. These tests cover the staleness guard added to
// close that gap.

describe("resolveCompactionStart", () => {
    const NOW = 1_800_000_000_000; // arbitrary fixed epoch ms

    function payload(overrides: Record<string, unknown> = {}) {
        return {
            trigger: "manual",
            sessionId: "sess-1",
            startedAt: new Date(NOW).toISOString(),
            ...overrides,
        };
    }

    it("accepts a fresh manual start", () => {
        const resolved = resolveCompactionStart(payload({ trigger: "manual" }), NOW);
        expect(resolved).toEqual({ trigger: "manual", startedAt: NOW });
    });

    it("accepts a fresh auto start", () => {
        const resolved = resolveCompactionStart(payload({ trigger: "auto" }), NOW);
        expect(resolved?.trigger).toBe("auto");
    });

    it("accepts a start still well within the plausible-duration window", () => {
        const fiveMinutesAgo = new Date(NOW - 5 * 60 * 1000).toISOString();
        const resolved = resolveCompactionStart(payload({ startedAt: fiveMinutesAgo }), NOW);
        expect(resolved).not.toBeNull();
    });

    it("rejects a stale replay older than the plausible-duration window", () => {
        // The exact bug this guard exists for: a compaction that
        // finished 20 minutes ago replays on reconnect and must not
        // resurrect a "Compacting…" state.
        const twentyMinutesAgo = new Date(NOW - 20 * 60 * 1000).toISOString();
        expect(resolveCompactionStart(payload({ startedAt: twentyMinutesAgo }), NOW)).toBeNull();
    });

    it("rejects right at the boundary consistently (just over the max is stale)", () => {
        const justOver = new Date(NOW - (10 * 60 * 1000 + 1)).toISOString();
        expect(resolveCompactionStart(payload({ startedAt: justOver }), NOW)).toBeNull();
    });

    it("accepts a startedAt slightly in the future within clock-skew tolerance, clamped to now", () => {
        const thirtySecondsFuture = new Date(NOW + 30 * 1000).toISOString();
        const resolved = resolveCompactionStart(payload({ startedAt: thirtySecondsFuture }), NOW);
        expect(resolved?.startedAt).toBe(NOW);
    });

    it("rejects a startedAt far in the future beyond clock-skew tolerance", () => {
        const fiveMinutesFuture = new Date(NOW + 5 * 60 * 1000).toISOString();
        expect(resolveCompactionStart(payload({ startedAt: fiveMinutesFuture }), NOW)).toBeNull();
    });

    it("rejects a missing startedAt (fail closed, not treated as fresh)", () => {
        const { startedAt, ...rest } = payload();
        expect(resolveCompactionStart(rest, NOW)).toBeNull();
    });

    it("rejects an unparseable startedAt", () => {
        expect(resolveCompactionStart(payload({ startedAt: "not-a-date" }), NOW)).toBeNull();
    });

    it("rejects an unrecognized trigger value", () => {
        expect(resolveCompactionStart(payload({ trigger: "sometimes" }), NOW)).toBeNull();
    });

    it("rejects a missing trigger", () => {
        const { trigger, ...rest } = payload();
        expect(resolveCompactionStart(rest, NOW)).toBeNull();
    });

    it("rejects a non-object payload", () => {
        expect(resolveCompactionStart(null, NOW)).toBeNull();
        expect(resolveCompactionStart(undefined, NOW)).toBeNull();
        expect(resolveCompactionStart("not-an-object", NOW)).toBeNull();
    });
});

describe("wasCompactionStartedAccepted", () => {
    // reagent P1 on PR #2378 (round 6): the reducer's round-5 fix makes
    // CompactionStarted a no-op (empty events) when a stray ping races
    // past the turn's own TurnEnd. Without checking dispatchPane's return
    // value, the caller would push a permanent "Compacting…" transcript
    // node for a compaction that isn't actually happening.

    it("returns true when the reducer accepted the transition", () => {
        expect(wasCompactionStartedAccepted([{ type: "compaction-started", trigger: "manual" }])).toBe(true);
    });

    it("returns false when the reducer rejected it as a no-op (empty events)", () => {
        expect(wasCompactionStartedAccepted([])).toBe(false);
    });

    it("returns false when only unrelated events are present", () => {
        expect(wasCompactionStartedAccepted([{ type: "tokens-updated", input: 1, output: null }])).toBe(false);
    });
});

describe("useCompactionStream — missed-ping retry (SPEC_COMPACTION_STARTED_RECONCILIATION_RACE_2026_09_02)", () => {
    // Regression coverage for the "Working… disappears mid-compaction after a
    // load/resume" bug: `compaction_started` can arrive over its own WPS
    // transport BEFORE `ReconcileTurnActive` has promoted a freshly-mounted
    // pane's `turnPhase` out of the mount-default `Idle`. The reducer's own
    // (deliberately strict — see reducer.ts's round-5 comment on
    // `CompactionStarted`) `workingFromPhase` gate correctly rejects that
    // first attempt; without a retry the ping is lost forever, since this WPS
    // channel is `persist: 0` and never replayed. These tests exercise the
    // hook's local missed-ping buffer + retry-on-promotion, not the reducer
    // gate itself (that's reducer.test.ts's job) — the fake `dispatchPane`
    // below mirrors only the one gate condition this fix's retry depends on.

    const IDLE: TurnPhase = { kind: "Idle" };
    const STREAMING: TurnPhase = { kind: "Streaming", bufferSize: 0, toolsActive: 0, lastEventMs: 0 };

    function makeFakeModel(turnPhase: () => TurnPhase) {
        const dispatchPane = vi.fn((command: { type: string; trigger?: "manual" | "auto" }) => {
            if (command.type !== "CompactionStarted") return [];
            // Mirrors reducer.ts's CompactionStarted `workingFromPhase` gate —
            // the one condition this fix's retry is designed around.
            const working = turnPhase().kind === "Streaming" || turnPhase().kind === "Submitting"
                || turnPhase().kind === "Interrupting";
            return working ? [{ type: "compaction-started", trigger: command.trigger }] : [];
        });
        return { blockId: "b", disposed: false, dispatchPane, dispatchDoc: vi.fn(() => []) } as any;
    }

    function makeFakeQueue() {
        return { pushNewNode: vi.fn(), scheduleFlush: vi.fn() } as any;
    }

    function fireCompactionStarted(startedAtIso: string, trigger: "manual" | "auto" = "auto") {
        const handler = [...hub.handlers.values()][0];
        if (!handler) throw new Error("no compaction_started handler registered — useCompactionStream did not mount");
        handler({ data: { trigger, startedAt: startedAtIso } });
    }

    beforeEach(() => {
        hub.handlers.clear();
        vi.useFakeTimers();
        vi.setSystemTime(1_800_000_000_000);
    });
    afterEach(() => vi.useRealTimers());

    it("buffers a ping rejected while Idle and retries once the pane is confirmed working", async () => {
        await createRoot(async (dispose) => {
            const [turnPhase, setTurnPhase] = createSignal<TurnPhase>(IDLE);
            const model = makeFakeModel(turnPhase);
            const queue = makeFakeQueue();
            useCompactionStream({
                blockId: "b",
                model,
                queue,
                hasNodeId: () => false,
                addNodeId: () => {},
                turnPhase,
            } as any);
            await Promise.resolve(); // flush the retry effect's initial run

            fireCompactionStarted(new Date().toISOString());
            // First attempt: rejected (still Idle) — no transcript node pushed.
            expect(model.dispatchPane).toHaveBeenCalledTimes(1);
            expect(queue.pushNewNode).not.toHaveBeenCalled();

            // ReconcileTurnActive resolves — the pane is now confirmed working.
            setTurnPhase(STREAMING);
            await Promise.resolve(); // flush the retry effect's re-run

            // Retry fires automatically: same command re-dispatched, this time
            // accepted, and the transcript node is pushed (previously lost forever).
            expect(model.dispatchPane).toHaveBeenCalledTimes(2);
            expect(model.dispatchPane).toHaveBeenLastCalledWith(
                expect.objectContaining({ type: "CompactionStarted" }),
            );
            expect(queue.pushNewNode).toHaveBeenCalledTimes(1);

            dispose();
        });
    });

    it("retries at most once per missed ping, even if turnPhase flips working again later", async () => {
        await createRoot(async (dispose) => {
            const [turnPhase, setTurnPhase] = createSignal<TurnPhase>(IDLE);
            const model = makeFakeModel(turnPhase);
            const queue = makeFakeQueue();
            useCompactionStream({
                blockId: "b",
                model,
                queue,
                hasNodeId: () => false,
                addNodeId: () => {},
                turnPhase,
            } as any);
            await Promise.resolve();

            fireCompactionStarted(new Date().toISOString());
            expect(model.dispatchPane).toHaveBeenCalledTimes(1);

            setTurnPhase(STREAMING); // consumes the missed ping (retry #1, accepted)
            await Promise.resolve();
            expect(model.dispatchPane).toHaveBeenCalledTimes(2);

            setTurnPhase(IDLE);
            setTurnPhase(STREAMING); // a later, unrelated working transition
            await Promise.resolve();
            // No missed ping left to retry — dispatch count must not grow again.
            expect(model.dispatchPane).toHaveBeenCalledTimes(2);

            dispose();
        });
    });

    it("does not dispatch anything extra when the pane becomes working with no missed ping", async () => {
        await createRoot(async (dispose) => {
            const [turnPhase, setTurnPhase] = createSignal<TurnPhase>(IDLE);
            const model = makeFakeModel(turnPhase);
            const queue = makeFakeQueue();
            useCompactionStream({
                blockId: "b",
                model,
                queue,
                hasNodeId: () => false,
                addNodeId: () => {},
                turnPhase,
            } as any);
            await Promise.resolve();

            setTurnPhase(STREAMING);
            await Promise.resolve();
            expect(model.dispatchPane).not.toHaveBeenCalled();

            dispose();
        });
    });

    it("does not retry a ping the reducer rejected while ALREADY working (genuinely stale, not early)", async () => {
        // Round-6-style case: the reducer can also reject for reasons other
        // than "not working yet" (e.g. isStaleVsLastBoundary). This hook has
        // no way to distinguish that from the "too early" case by the reject
        // alone — it buffers either way — but the retry effect only fires on
        // an Idle/Disconnected/Done → working PROMOTION. If the pane is
        // already working when the ping is rejected, staying working never
        // re-triggers the effect, so no spurious retry loop occurs.
        await createRoot(async (dispose) => {
            const [turnPhase] = createSignal<TurnPhase>(STREAMING);
            const alwaysReject = vi.fn(() => []);
            const model = { blockId: "b", disposed: false, dispatchPane: alwaysReject, dispatchDoc: vi.fn(() => []) } as any;
            const queue = makeFakeQueue();
            useCompactionStream({
                blockId: "b",
                model,
                queue,
                hasNodeId: () => false,
                addNodeId: () => {},
                turnPhase,
            } as any);
            await Promise.resolve();

            fireCompactionStarted(new Date().toISOString());
            await Promise.resolve();
            expect(model.dispatchPane).toHaveBeenCalledTimes(1);

            // No further dispatch — turnPhase never transitions again.
            expect(model.dispatchPane).toHaveBeenCalledTimes(1);

            dispose();
        });
    });

    it("gives up after one failed retry — does not keep re-dispatching on every later turnPhase change (reagent P1)", async () => {
        // A buffered ping's retry can itself be rejected (e.g. its own
        // compact_boundary arrived while buffered — isStaleVsLastBoundary).
        // The retry effect depends on `turnPhase()`, which the real reducer
        // replaces on nearly every Streaming event (StreamFlushObserved
        // returns a new phase object per flush) — so if a failed retry left
        // `missedPing` set, the effect would re-fire and re-dispatch on
        // every subsequent stream chunk for the rest of the working
        // session, not just once. `missedPing` must be cleared after the
        // retry attempt regardless of outcome.
        await createRoot(async (dispose) => {
            const [turnPhase, setTurnPhase] = createSignal<TurnPhase>(IDLE);
            const alwaysReject = vi.fn(() => []);
            const model = { blockId: "b", disposed: false, dispatchPane: alwaysReject, dispatchDoc: vi.fn(() => []) } as any;
            const queue = makeFakeQueue();
            useCompactionStream({
                blockId: "b",
                model,
                queue,
                hasNodeId: () => false,
                addNodeId: () => {},
                turnPhase,
            } as any);
            await Promise.resolve();

            fireCompactionStarted(new Date().toISOString());
            expect(model.dispatchPane).toHaveBeenCalledTimes(1); // live delivery: rejected, buffered

            setTurnPhase({ ...STREAMING, lastEventMs: 1 }); // promotion → retry fires
            await Promise.resolve();
            expect(model.dispatchPane).toHaveBeenCalledTimes(2); // retry: rejected again, must NOT re-buffer

            // Simulate ordinary stream churn: the reducer replaces the
            // turnPhase object on nearly every event while Streaming, with
            // no further compaction ping involved at all.
            for (let i = 2; i < 6; i++) {
                setTurnPhase({ ...STREAMING, lastEventMs: i });
                await Promise.resolve();
            }
            expect(model.dispatchPane).toHaveBeenCalledTimes(2); // no growth — the buggy version kept climbing here

            dispose();
        });
    });

    it("drops an expired buffered ping instead of replaying it into a later, unrelated turn (codex P2)", async () => {
        // The exact scenario codex flagged: a ping that was actually
        // genuinely stale (its turn already ended before it arrived) gets
        // buffered on live delivery. The confirming
        // ReconcileTurnActive(active: false) never promotes the phase (it's
        // already Idle — a same-ref no-op), so there is no reactive signal
        // to clear `missedPing` at that point. Without a bound, the retry
        // effect would only fire on whatever working-phase transition comes
        // NEXT — which could be a completely unrelated TurnStart from the
        // user's next message — and re-dispatch the stale ping against it.
        await createRoot(async (dispose) => {
            const [turnPhase, setTurnPhase] = createSignal<TurnPhase>(IDLE);
            const model = makeFakeModel(turnPhase);
            const queue = makeFakeQueue();
            useCompactionStream({
                blockId: "b",
                model,
                queue,
                hasNodeId: () => false,
                addNodeId: () => {},
                turnPhase,
            } as any);
            await Promise.resolve();

            fireCompactionStarted(new Date().toISOString());
            expect(model.dispatchPane).toHaveBeenCalledTimes(1); // rejected (Idle), buffered

            // Time passes with turnPhase never changing (the inactive
            // confirmation was a same-ref no-op) — well past the retry window.
            vi.advanceTimersByTime(MISSED_PING_RETRY_WINDOW_MS + 1000);

            // An entirely unrelated new turn starts (e.g. the user's next message).
            setTurnPhase({ kind: "Submitting", submittedAt: Date.now(), pendingContent: "hi" });
            await Promise.resolve();

            // The expired ping must be dropped, not replayed into this new turn.
            expect(model.dispatchPane).toHaveBeenCalledTimes(1);
            expect(queue.pushNewNode).not.toHaveBeenCalled();

            dispose();
        });
    });
});
