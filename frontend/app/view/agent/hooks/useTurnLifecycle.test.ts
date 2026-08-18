// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    handlers: new Map<string, (e: unknown) => void>(),
    // Every handler ever registered for "agent-message-accepted", in
    // registration order — including ones already unsubscribed — so tests
    // can grab a STALE reference and invoke it directly, simulating a
    // transport-level "delivered after unsubscribe" race independent of
    // whatever this mock's own Map bookkeeping does.
    acceptedHistory: [] as Array<(e: unknown) => void>,
}));

vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: vi.fn((sub: { eventType: string; handler: (e: unknown) => void }) => {
        hub.handlers.set(sub.eventType, sub.handler);
        if (sub.eventType === "agent-message-accepted") hub.acceptedHistory.push(sub.handler);
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
        hub.acceptedHistory.length = 0;
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

    // reagentx P1 (round 2, on the acceptance-cancels-timer fix itself): a
    // STALE accepted event for an EARLIER episode — one whose own listener
    // was already unsubscribed — must not be able to disarm a NEWER
    // episode's timer even if something outside this hook's control (a
    // transport-level race) still manages to invoke that old handler
    // reference directly. Captures episode A's handler from
    // `acceptedHistory` (bypassing the mock's current-handler lookup
    // entirely, which would just find episode B's fresh one), lets episode
    // A time out normally, starts an unrelated episode B, then fires the
    // STALE captured reference directly.
    it("a stale accepted-event handler captured from an earlier, already-timed-out episode cannot disarm a later episode's timer", () => {
        let setPhase!: (p: TurnPhase) => void;
        createRoot((d) => {
            dispose = d;
            const [getPhase, setP] = createSignal<TurnPhase>({ kind: "Idle" });
            setPhase = setP;
            useTurnLifecycle(mkOpts(getPhase, setP));
        });

        setPhase({ kind: "Submitting", submittedAt: 1000, pendingContent: "" } as TurnPhase);
        expect(hub.acceptedHistory).toHaveLength(1);
        const staleHandlerForEpisodeA = hub.acceptedHistory[0];

        // Episode A genuinely never gets accepted and times out normally.
        vi.advanceTimersByTime(SUBMIT_TIMEOUT_MS);
        expect(dispatchPane).toHaveBeenCalledWith(expect.objectContaining({ type: "SubmitTimeoutElapsed" }));
        dispatchPane.mockClear();

        // User retries — a brand-new episode B, unrelated to A, whose own
        // acceptance genuinely never arrives either.
        setPhase({ kind: "Done", outcome: "errored", finishedAt: Date.now() } as TurnPhase);
        setPhase({ kind: "Submitting", submittedAt: 2000, pendingContent: "" } as TurnPhase);
        expect(hub.acceptedHistory).toHaveLength(2);

        // A's stale, already-unsubscribed handler fires anyway (simulating
        // a delivery race this hook can't prevent at the transport layer).
        staleHandlerForEpisodeA({ data: { message_id: "msg-A" } });

        // B's own timer must be unaffected — it still times out normally.
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

    // SPEC_AGENT_TURN_PHASE_TIMELINE_LOGGING_2026_08_18.md: log every
    // visibilitychange firing (both directions), not just the "visible"
    // branch that dispatches the catch-up tick — this is the only direct
    // evidence the listener fired at all for a given pane, which the fix
    // above depends on but which nothing confirmed before this.
    it("logs [wave-turn] visibility: hidden→visible when the document becomes visible", () => {
        createRoot((d) => {
            dispose = d;
            const [getPhase, setP] = createSignal<TurnPhase>({ kind: "Idle" });
            useTurnLifecycle(mkOpts(getPhase, setP));
        });

        const info = vi.spyOn(console, "info").mockImplementation(() => {});
        try {
            Object.defineProperty(document, "visibilityState", { value: "visible", configurable: true });
            document.dispatchEvent(new Event("visibilitychange"));

            const line = info.mock.calls.find((c) => c[0] === "[wave-turn]");
            expect(line).toBeDefined();
            expect(line?.join(" ")).toContain("visibility: hidden→visible");
        } finally {
            info.mockRestore();
        }
    });

    it("logs [wave-turn] visibility: visible→hidden when the document becomes hidden", () => {
        createRoot((d) => {
            dispose = d;
            const [getPhase, setP] = createSignal<TurnPhase>({ kind: "Idle" });
            useTurnLifecycle(mkOpts(getPhase, setP));
        });

        const info = vi.spyOn(console, "info").mockImplementation(() => {});
        try {
            Object.defineProperty(document, "visibilityState", { value: "hidden", configurable: true });
            document.dispatchEvent(new Event("visibilitychange"));

            const line = info.mock.calls.find((c) => c[0] === "[wave-turn]");
            expect(line).toBeDefined();
            expect(line?.join(" ")).toContain("visibility: visible→hidden");
        } finally {
            info.mockRestore();
        }
    });
});
