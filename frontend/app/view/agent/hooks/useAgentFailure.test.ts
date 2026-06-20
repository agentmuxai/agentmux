// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * useAgentFailure auto-retry budget (§6). Drives the exact wave-event order the
 * backend emits — a failing transient turn publishes `controllerstatus done`
 * with `shellprocexitcode: 0` and THEN `agentfailure` (subprocess.rs) — to pin
 * the two invariants reagent flagged on #1485:
 *   1. a sustained transient failure auto-retries at most twice, then caps
 *      (no infinite hammer), even though every failing turn exits 0;
 *   2. the budget is restored only on a genuine success (a `done` with no
 *      `agentfailure` after it), so a later unrelated transient still gets 2.
 */

import { createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

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

const transient = (): AgentFailure => ({ code: "rate_limited", title: "Throttled", detail: "429", retryable: true });

const fire = (type: string, data: unknown) => {
    const h = hub.handlers.get(type);
    if (!h) throw new Error(`no "${type}" handler registered — useAgentFailure onMount did not run`);
    h({ data });
};
// A failing transient turn: running → done(exit 0) → agentfailure (the order
// the backend emits; exit 0 because the cause came from an error result frame).
const failingTurn = () => {
    fire("controllerstatus", { shellprocstatus: "running" });
    fire("controllerstatus", { shellprocstatus: "done", shellprocexitcode: 0 });
    fire("agentfailure", transient());
};
// A turn that completes successfully: running → done(exit 0), no agentfailure.
const successfulTurn = () => {
    fire("controllerstatus", { shellprocstatus: "running" });
    fire("controllerstatus", { shellprocstatus: "done", shellprocexitcode: 0 });
};
const hasRetryCountdown = (ui: UseAgentFailureResult) =>
    (ui.row()?.actions ?? []).some((a) => a.label === "Retry now (5s)");

const mkUI = (onRetry: () => void): UseAgentFailureResult =>
    useAgentFailure({ blockId: "b", onRetry, onLoginAgain() {}, onUseExistingLogin() {}, onTrustCenter() {}, onNewSession() {} });

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
            const ui = mkUI(vi.fn());
            await Promise.resolve(); // flush onMount
            expect(ui.row()).not.toBeNull();
            expect(ui.row()!.title).toBe("Not authenticated");
            dispose();
        });
    });

    it("shows no failure row when agent:last_failure meta is null", async () => {
        hub.persistedFailure = null;
        await createRoot(async (dispose) => {
            const ui = mkUI(vi.fn());
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
        vi.useFakeTimers();
    });
    afterEach(() => vi.useRealTimers());

    it("auto-retries a sustained transient failure exactly twice, then caps", async () => {
        await createRoot(async (dispose) => {
            const onRetry = vi.fn();
            const ui = mkUI(onRetry);
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

    it("a failing turn's done(exit 0) does NOT reset the cap (reagent r3 regression)", async () => {
        await createRoot(async (dispose) => {
            const onRetry = vi.fn();
            mkUI(onRetry);
            await Promise.resolve();

            failingTurn(); vi.advanceTimersByTime(5000);
            failingTurn(); vi.advanceTimersByTime(10000);
            failingTurn(); // each emitted done(exit 0) before its agentfailure
            vi.advanceTimersByTime(60000);
            expect(onRetry).toHaveBeenCalledTimes(2); // exit-0 dones never reset the budget

            dispose();
        });
    });

    it("restores the full budget after a genuine success", async () => {
        await createRoot(async (dispose) => {
            const onRetry = vi.fn();
            const ui = mkUI(onRetry);
            await Promise.resolve();

            // Burn the budget to the cap.
            failingTurn(); vi.advanceTimersByTime(5000);
            failingTurn(); vi.advanceTimersByTime(10000);
            expect(onRetry).toHaveBeenCalledTimes(2);

            // A turn succeeds (done, no agentfailure); the next turn starting
            // confirms it → budget reset.
            successfulTurn();
            fire("controllerstatus", { shellprocstatus: "running" });

            // A fresh transient failure now auto-retries again on a full budget.
            fire("controllerstatus", { shellprocstatus: "done", shellprocexitcode: 0 });
            fire("agentfailure", transient());
            expect(hasRetryCountdown(ui)).toBe(true);
            vi.advanceTimersByTime(5000);
            expect(onRetry).toHaveBeenCalledTimes(3);

            dispose();
        });
    });
});
