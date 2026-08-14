// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    handlers: new Map<string, (e: unknown) => void>(),
}));

vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: vi.fn((sub: { eventType: string; handler: (e: unknown) => void }) => {
        hub.handlers.set(sub.eventType, sub.handler);
        return () => {
            hub.handlers.delete(sub.eventType);
        };
    }),
}));
vi.mock("@/app/store/wos", () => ({ makeORef: (a: string, b: string) => `${a}:${b}` }));
vi.mock("@/app/store/agent-pane-state-store", () => ({ snapshot: () => null }));
vi.mock("@/store/token-usage", () => ({ recordTurn: () => {} }));

import { useTurnLifecycle } from "./useTurnLifecycle";
import { SUBMIT_TIMEOUT_MS, type TurnPhase } from "@/app/store/agent-pane-state/types";

const fireAgentMessageAccepted = () => {
    const handler = hub.handlers.get("agent-message-accepted");
    if (!handler) throw new Error("agent-message-accepted handler not registered — onMount did not run");
    handler({ data: { message_id: "msg-1" } });
};

// Regression coverage for
// docs/reports/REPORT_AGENTA_STUCK_WORKING_INVESTIGATION_2026_08_14.md — two
// dispatch-side gaps found live: (1) `SubmitTimeoutElapsed` had no consumer
// anywhere in the app (PR D #994 shipped the reducer half only and deferred
// the dispatch-side setTimeout to a follow-up that never landed), and (2) the
// stream watchdog interval showed zero evidence of ticking during a 29+
// minute stuck-Working incident, with component-unmount and a reducer bug
// both ruled out — the fix adds a `document.visibilitychange`-triggered
// catch-up tick as a backstop.
describe("useTurnLifecycle — dispatch-side timeout wiring", () => {
    let dispatchPane: ReturnType<typeof vi.fn>;
    let dispose: () => void;

    const mkOpts = (getTurnPhase: () => TurnPhase, setTurnPhase: (p: TurnPhase) => void) => ({
        blockId: "block-1",
        model: { dispatchPane } as any,
        turnPhaseAtom: [getTurnPhase, setTurnPhase] as any,
        queue: { pushNewNode: vi.fn(), scheduleFlush: vi.fn() } as any,
        flushParserPending: vi.fn(),
        hasNodeId: () => false,
        addNodeId: vi.fn(),
    });

    beforeEach(() => {
        vi.useFakeTimers();
        dispatchPane = vi.fn();
        hub.handlers.clear();
    });

    afterEach(() => {
        dispose?.();
        vi.useRealTimers();
    });

    it("dispatches SubmitTimeoutElapsed after SUBMIT_TIMEOUT_MS if still Submitting", () => {
        let setPhase!: (p: TurnPhase) => void;
        createRoot((d) => {
            dispose = d;
            const [getPhase, setP] = createSignal<TurnPhase>({ kind: "Idle" });
            setPhase = setP;
            useTurnLifecycle(mkOpts(getPhase, setP));
        });

        setPhase({ kind: "Submitting", submittedAt: Date.now(), pendingContent: "" } as TurnPhase);
        expect(dispatchPane).not.toHaveBeenCalledWith(expect.objectContaining({ type: "SubmitTimeoutElapsed" }));

        vi.advanceTimersByTime(SUBMIT_TIMEOUT_MS - 1);
        expect(dispatchPane).not.toHaveBeenCalledWith(expect.objectContaining({ type: "SubmitTimeoutElapsed" }));

        vi.advanceTimersByTime(1);
        expect(dispatchPane).toHaveBeenCalledWith(expect.objectContaining({ type: "SubmitTimeoutElapsed" }));
    });

    it("does NOT dispatch SubmitTimeoutElapsed if the phase moved off Submitting before the deadline", () => {
        let setPhase!: (p: TurnPhase) => void;
        createRoot((d) => {
            dispose = d;
            const [getPhase, setP] = createSignal<TurnPhase>({ kind: "Idle" });
            setPhase = setP;
            useTurnLifecycle(mkOpts(getPhase, setP));
        });

        setPhase({ kind: "Submitting", submittedAt: Date.now(), pendingContent: "" } as TurnPhase);
        vi.advanceTimersByTime(SUBMIT_TIMEOUT_MS / 2);
        setPhase({ kind: "Streaming", bufferSize: 0, toolsActive: 0, lastEventMs: Date.now() } as TurnPhase);

        vi.advanceTimersByTime(SUBMIT_TIMEOUT_MS);
        expect(dispatchPane).not.toHaveBeenCalledWith(expect.objectContaining({ type: "SubmitTimeoutElapsed" }));
    });

    it("re-arms independently on a second Submitting episode (not a one-shot)", () => {
        let setPhase!: (p: TurnPhase) => void;
        createRoot((d) => {
            dispose = d;
            const [getPhase, setP] = createSignal<TurnPhase>({ kind: "Idle" });
            setPhase = setP;
            useTurnLifecycle(mkOpts(getPhase, setP));
        });

        setPhase({ kind: "Submitting", submittedAt: Date.now(), pendingContent: "" } as TurnPhase);
        setPhase({ kind: "Done", outcome: "completed", finishedAt: Date.now() } as TurnPhase);
        setPhase({ kind: "Submitting", submittedAt: Date.now(), pendingContent: "" } as TurnPhase);

        vi.advanceTimersByTime(SUBMIT_TIMEOUT_MS);
        expect(dispatchPane).toHaveBeenCalledWith(expect.objectContaining({ type: "SubmitTimeoutElapsed" }));
        expect(
            dispatchPane.mock.calls.filter((c) => c[0]?.type === "SubmitTimeoutElapsed"),
        ).toHaveLength(1);
    });

    // codex P1 on PR #2575: usePendingMessageAcceptance.ts deliberately
    // leaves an idle send in Submitting once AgentMessageAccepted arrives
    // (re-dispatching TurnStart there would re-arm this exact timer
    // unnecessarily), and a backend-accepted turn can legitimately take
    // well over 30s to produce its first token. A blind timeout would
    // misfire on that healthy case and — since neither StreamFlushObserved
    // nor bumpEvent re-promote an errored Done phase — orphan the eventual
    // real response. AgentMessageAccepted must cancel the timer.
    it("does NOT dispatch SubmitTimeoutElapsed if the backend accepts the message before the deadline, even well past SUBMIT_TIMEOUT_MS", () => {
        let setPhase!: (p: TurnPhase) => void;
        createRoot((d) => {
            dispose = d;
            const [getPhase, setP] = createSignal<TurnPhase>({ kind: "Idle" });
            setPhase = setP;
            useTurnLifecycle(mkOpts(getPhase, setP));
        });

        setPhase({ kind: "Submitting", submittedAt: Date.now(), pendingContent: "" } as TurnPhase);
        vi.advanceTimersByTime(SUBMIT_TIMEOUT_MS / 2);
        fireAgentMessageAccepted();

        // Phase stays Submitting (the accepted-but-not-yet-streaming case
        // usePendingMessageAcceptance.ts intentionally leaves untouched) —
        // simulate a slow-but-healthy turn that takes well over the
        // original 30s bound to produce its first token.
        vi.advanceTimersByTime(SUBMIT_TIMEOUT_MS * 3);
        expect(dispatchPane).not.toHaveBeenCalledWith(expect.objectContaining({ type: "SubmitTimeoutElapsed" }));
    });

    it("still times out a LATER Submitting episode normally after an earlier one was accepted", () => {
        let setPhase!: (p: TurnPhase) => void;
        createRoot((d) => {
            dispose = d;
            const [getPhase, setP] = createSignal<TurnPhase>({ kind: "Idle" });
            setPhase = setP;
            useTurnLifecycle(mkOpts(getPhase, setP));
        });

        setPhase({ kind: "Submitting", submittedAt: Date.now(), pendingContent: "" } as TurnPhase);
        fireAgentMessageAccepted();
        setPhase({ kind: "Streaming", bufferSize: 0, toolsActive: 0, lastEventMs: Date.now() } as TurnPhase);
        setPhase({ kind: "Done", outcome: "completed", finishedAt: Date.now() } as TurnPhase);

        // A brand-new send whose RPC genuinely never reaches the backend —
        // no AgentMessageAccepted this time — must still be caught.
        setPhase({ kind: "Submitting", submittedAt: Date.now(), pendingContent: "" } as TurnPhase);
        vi.advanceTimersByTime(SUBMIT_TIMEOUT_MS);
        expect(dispatchPane).toHaveBeenCalledWith(expect.objectContaining({ type: "SubmitTimeoutElapsed" }));
    });

    it("dispatches an extra StreamWatchdogTick when the document becomes visible again", () => {
        createRoot((d) => {
            dispose = d;
            const [getPhase, setP] = createSignal<TurnPhase>({ kind: "Idle" });
            useTurnLifecycle(mkOpts(getPhase, setP));
        });

        dispatchPane.mockClear();
        Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
        document.dispatchEvent(new Event("visibilitychange"));

        expect(dispatchPane).toHaveBeenCalledWith(expect.objectContaining({ type: "StreamWatchdogTick" }));
    });

    it("does NOT dispatch a tick when the document becomes hidden", () => {
        createRoot((d) => {
            dispose = d;
            const [getPhase, setP] = createSignal<TurnPhase>({ kind: "Idle" });
            useTurnLifecycle(mkOpts(getPhase, setP));
        });

        dispatchPane.mockClear();
        Object.defineProperty(document, "visibilityState", { value: "hidden", configurable: true });
        document.dispatchEvent(new Event("visibilitychange"));

        expect(dispatchPane).not.toHaveBeenCalledWith(expect.objectContaining({ type: "StreamWatchdogTick" }));
    });

    it("removes the visibilitychange listener on cleanup (no dispatch after dispose)", () => {
        createRoot((d) => {
            dispose = d;
            const [getPhase, setP] = createSignal<TurnPhase>({ kind: "Idle" });
            useTurnLifecycle(mkOpts(getPhase, setP));
        });

        dispose();
        dispatchPane.mockClear();
        Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
        document.dispatchEvent(new Event("visibilitychange"));

        expect(dispatchPane).not.toHaveBeenCalled();
    });
});
