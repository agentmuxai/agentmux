// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentFailure auto-retry budget (§6). Pins the invariants reagent
 * flagged on #1485 and #1987:
 *   1. a sustained transient failure auto-retries at most twice, then caps
 *      (no infinite hammer);
 *   2. the budget is restored after a genuine turn success (`turn-ended`
 *      with outcome "completed" — the reducer's own authoritative verdict,
 *      not an inference from raw process-exit timing), so a later
 *      unrelated transient still gets 2 (#1485);
 *   3. the budget is ALSO restored when the user composes and sends a
 *      genuinely fresh message while the failure row is still showing,
 *      bypassing Retry (simulated here as an external `state.failure`
 *      clear the hook did not itself initiate) — reagent P1 on #1987
 *      found the previous ControllerStatus-based check for this was dead
 *      code in practice (TurnStart clears `state.failure` synchronously,
 *      long before the backend's async event this hook used to wait for
 *      could ever round-trip), which meant persistent-mode agents — the
 *      exact case #1987 targets — could NEVER get their budget reset by
 *      this path.
 */

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PaneFailure } from "@/app/store/agent-pane-state/types";
import type { AgentPaneEvent } from "@/app/store/agent-pane-state-store";
import { __resetListeners, fireEvent } from "@/app/store/agent-pane-state-store";

const hub = vi.hoisted(() => ({
    handlers: new Map<string, (e: unknown) => void>(),
    persistedFailure: null as AgentFailure | null,
}));

vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: vi.fn((sub: { eventType: string; handler: (e: unknown) => void }) => {
        hub.handlers.set(sub.eventType, sub.handler);
        return () => hub.handlers.delete(sub.eventType);
    }),
}));
vi.mock("@/app/store/wos", () => ({ makeORef: (a: string, b: string) => `${a}:${b}` }));
vi.mock("@/app/store/global", () => ({
    getBlockMetaKeyAtom: (_blockId: string, _key: string) => () => hub.persistedFailure,
}));

import { useAgentFailure, type UseAgentFailureResult } from "./useAgentFailure";

const BLOCK_ID = "b";
const transient = (): AgentFailure => ({ code: "rate_limited", title: "Throttled", detail: "429", retryable: true });

const fire = (type: string, data: unknown) => {
    const h = hub.handlers.get(type);
    if (!h) throw new Error(`no "${type}" handler registered — useAgentFailure onMount did not run`);
    h({ data });
};
const failingTurn = () => fire("agentfailure", transient());
// The reducer's own authoritative "this turn genuinely succeeded" verdict
// (real agent-pane-state-store multicast, matching production wiring —
// see `unsubTurnEnded` in useAgentFailure.ts). A failed turn goes through
// FailureObserved instead and never emits this, so it can't misfire here.
const successfulTurnEnded = () =>
    fireEvent(BLOCK_ID, { type: "turn-ended", outcome: "completed", statsMerged: false, stoppingCleared: false });
const hasRetryCountdown = (ui: UseAgentFailureResult) =>
    (ui.row()?.actions ?? []).some((a) => a.label === "Retry now (5s)");

// Minimal fake AgentPaneModel: dispatchPane mirrors ONLY the two commands
// this hook actually sends (FailureObserved / FailureCleared) against a
// local signal — a faithful stand-in for the real reducer's `state.failure`
// field for the purposes of this hook-level test (no TurnStart/turnPhase
// involved here; that's covered by reducer.test.ts).
const makeFakeModel = () => {
    const [failure, setFailure] = createSignal<PaneFailure | null>(null);
    const model = {
        blockId: BLOCK_ID,
        disposed: false,
        dispatchPane: vi.fn((command: { type: string; failure?: AgentFailure; at?: number }) => {
            if (command.type === "FailureObserved") {
                setFailure({ data: command.failure!, at: command.at! });
            } else if (command.type === "FailureCleared") {
                setFailure(null);
            }
            return [];
        }),
        dispatchDoc: vi.fn(() => []),
    };
    return { model: model as any, failure };
};

/** Simulates the reducer's TurnStart implicitly clearing a pre-existing
 *  `state.failure` — i.e. a fresh message sent OUTSIDE this hook's own
 *  `doRetry`, bypassing Retry entirely. Calling `model.dispatchPane`
 *  directly (rather than through the hook's `row().actions` Retry button)
 *  is exactly what makes this "external" from the hook's point of view. */
const simulateFreshTurnStartClearingFailure = (model: { dispatchPane: (c: unknown) => AgentPaneEvent[] }) =>
    model.dispatchPane({ type: "FailureCleared" });

const mkUI = (onRetry: () => void) => {
    const { model, failure } = makeFakeModel();
    const ui = useAgentFailure({
        blockId: BLOCK_ID,
        model,
        failure,
        onRetry,
        onLoginAgain() {},
        onUseExistingLogin() {},
        onLoginViaTerminal() {},
        onOpenArmory() {},
        onNewSession() {},
    });
    return { ui, model };
};

describe("useAgentFailure P1.2 — persisted meta seed on mount", () => {
    beforeEach(() => {
        hub.handlers.clear();
        hub.persistedFailure = null;
        vi.useFakeTimers();
    });
    afterEach(() => vi.useRealTimers());

    it("seeds failure row from block meta agent:last_failure on mount", async () => {
        const authFailure: AgentFailure = {
            code: "auth",
            title: "Not authenticated",
            detail: "401 Invalid authentication credentials",
            retryable: false,
        };
        hub.persistedFailure = authFailure;
        await createRoot(async (dispose) => {
            const { ui } = mkUI(vi.fn());
            await Promise.resolve(); // flush onMount
            expect(ui.row()).not.toBeNull();
            expect(ui.row()!.title).toBe("Not authenticated");
            dispose();
        });
    });

    it("shows no failure row when agent:last_failure meta is null", async () => {
        hub.persistedFailure = null;
        await createRoot(async (dispose) => {
            const { ui } = mkUI(vi.fn());
            await Promise.resolve();
            expect(ui.row()).toBeNull();
            dispose();
        });
    });
});

describe("useAgentFailure auto-retry budget (§6)", () => {
    beforeEach(() => {
        hub.handlers.clear();
        hub.persistedFailure = null;
        __resetListeners();
        vi.useFakeTimers();
    });
    afterEach(() => vi.useRealTimers());

    it("auto-retries a sustained transient failure exactly twice, then caps", async () => {
        await createRoot(async (dispose) => {
            const onRetry = vi.fn();
            const { ui } = mkUI(onRetry);
            await Promise.resolve(); // flush onMount subscriptions

            failingTurn(); // failure 1 → arm 5s
            expect(hasRetryCountdown(ui)).toBe(true);
            vi.advanceTimersByTime(5000); // auto-retry 1
            expect(onRetry).toHaveBeenCalledTimes(1);

            failingTurn(); // failure 2 → arm 10s
            vi.advanceTimersByTime(10000); // auto-retry 2
            expect(onRetry).toHaveBeenCalledTimes(2);

            failingTurn(); // failure 3 → capped (no countdown)
            vi.advanceTimersByTime(60000);
            expect(onRetry).toHaveBeenCalledTimes(2); // cap holds — no 3rd auto-retry
            // Manual retry still offered (label without countdown).
            expect((ui.row()?.actions ?? []).some((a) => a.label === "Retry now")).toBe(true);

            dispose();
        });
    });

    it("a sustained run of failures does NOT reset the cap", async () => {
        await createRoot(async (dispose) => {
            const onRetry = vi.fn();
            mkUI(onRetry);
            await Promise.resolve();

            failingTurn(); vi.advanceTimersByTime(5000);
            failingTurn(); vi.advanceTimersByTime(10000);
            failingTurn();
            vi.advanceTimersByTime(60000);
            expect(onRetry).toHaveBeenCalledTimes(2); // repeated failures never reset the budget

            dispose();
        });
    });

    it("restores the full budget after a genuine turn success (turn-ended outcome completed)", async () => {
        await createRoot(async (dispose) => {
            const onRetry = vi.fn();
            const { ui } = mkUI(onRetry);
            await Promise.resolve();

            // Burn the budget to the cap.
            failingTurn(); vi.advanceTimersByTime(5000);
            failingTurn(); vi.advanceTimersByTime(10000);
            expect(onRetry).toHaveBeenCalledTimes(2);

            // The retried turn genuinely succeeds — the reducer's own
            // authoritative verdict, not an inference from process-exit timing.
            successfulTurnEnded();

            // A fresh transient failure now auto-retries again on a full budget.
            fire("agentfailure", transient());
            expect(hasRetryCountdown(ui)).toBe(true);
            vi.advanceTimersByTime(5000);
            expect(onRetry).toHaveBeenCalledTimes(3);

            dispose();
        });
    });

    it("restores the full budget when a fresh message is sent while the failure row is still showing (reagent P1 on #1987)", async () => {
        // Regression guard for reagent's finding that the previous
        // ControllerStatus-based check for this case was dead code in
        // practice — TurnStart clears state.failure synchronously, well
        // before the backend's async ControllerStatus:running event the old
        // check waited for could ever land. Persistent-mode agents (this
        // PR's whole target) never even emit that event between turns, so
        // the budget could never reset for them via that path. This test
        // exercises the replacement: the state.failure transition-effect,
        // triggered here by dispatching FailureCleared directly against the
        // model (bypassing the hook's own Retry button entirely) — exactly
        // what a fresh, unrelated TurnStart looks like from the hook's
        // point of view.
        await createRoot(async (dispose) => {
            const onRetry = vi.fn();
            const { ui, model } = mkUI(onRetry);
            await Promise.resolve();

            // Burn the budget to the cap.
            failingTurn(); vi.advanceTimersByTime(5000);
            failingTurn(); vi.advanceTimersByTime(10000);
            expect(onRetry).toHaveBeenCalledTimes(2);

            failingTurn(); // failure 3 → would be capped on the old budget
            expect(hasRetryCountdown(ui)).toBe(false);

            // A fresh message clears the row from OUTSIDE this hook (not via
            // doRetry) — simulates TurnStart's implicit clear.
            simulateFreshTurnStartClearingFailure(model);

            // A new transient failure now auto-retries again on a full budget.
            fire("agentfailure", transient());
            expect(hasRetryCountdown(ui)).toBe(true);
            vi.advanceTimersByTime(5000);
            expect(onRetry).toHaveBeenCalledTimes(3);

            dispose();
        });
    });

    it("does NOT reset the budget when Retry itself clears the row (same episode)", async () => {
        await createRoot(async (dispose) => {
            const onRetry = vi.fn();
            mkUI(onRetry);
            await Promise.resolve();

            failingTurn(); // failure 1 → arm 5s, autoRetries becomes 1
            vi.advanceTimersByTime(5000); // auto-retry 1 fires -> doRetry() clears via `clear()`
            expect(onRetry).toHaveBeenCalledTimes(1);

            failingTurn(); // failure 2 → must arm the SECOND tier (10s), not reset to first (5s)
            expect(onRetry).toHaveBeenCalledTimes(1);
            vi.advanceTimersByTime(5000); // if the budget had wrongly reset, this would already fire
            expect(onRetry).toHaveBeenCalledTimes(1);
            vi.advanceTimersByTime(5000); // completes the real 10s tier
            expect(onRetry).toHaveBeenCalledTimes(2);

            dispose();
        });
    });
});
