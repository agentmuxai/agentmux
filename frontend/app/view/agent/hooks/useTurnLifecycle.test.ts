// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: vi.fn(() => () => {}),
}));
vi.mock("@/app/store/wos", () => ({ makeORef: (a: string, b: string) => `${a}:${b}` }));
vi.mock("@/app/store/agent-pane-state-store", () => ({ snapshot: () => null }));
vi.mock("@/store/token-usage", () => ({ recordTurn: () => {} }));

import { useTurnLifecycle } from "./useTurnLifecycle";
import { SUBMIT_TIMEOUT_MS, type TurnPhase } from "@/app/store/agent-pane-state/types";

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
        setPhase({ kind: "Done", outcome: "success", finishedAt: Date.now() } as TurnPhase);
        setPhase({ kind: "Submitting", submittedAt: Date.now(), pendingContent: "" } as TurnPhase);

        vi.advanceTimersByTime(SUBMIT_TIMEOUT_MS);
        expect(dispatchPane).toHaveBeenCalledWith(expect.objectContaining({ type: "SubmitTimeoutElapsed" }));
        expect(
            dispatchPane.mock.calls.filter((c) => c[0]?.type === "SubmitTimeoutElapsed"),
        ).toHaveLength(1);
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
