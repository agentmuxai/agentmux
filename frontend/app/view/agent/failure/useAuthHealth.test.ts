// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { PaneFailure } from "@/app/store/agent-pane-state/types";
import { createRoot, createSignal } from "solid-js";
import { createStore } from "solid-js/store";
import { describe, expect, it, vi } from "vitest";
import { useAuthHealth, useSyntheticAuthRow } from "./useAuthHealth";

type Cmd = { type: string; failure?: PaneFailure["data"]; turnAttempted?: boolean };

/** A pane model reduced to `state.failure` and a dispatch that applies the two failure commands. */
function fakePane(initial: PaneFailure | null = null) {
    const [state, setState] = createStore<{ failure: PaneFailure | null }>({ failure: initial });
    const dispatched: Cmd[] = [];
    const dispatchPane = (cmd: Cmd) => {
        dispatched.push(cmd);
        if (cmd.type === "FailureObserved") {
            setState("failure", { data: cmd.failure!, at: 0, turnAttempted: cmd.turnAttempted ?? true } as PaneFailure);
        } else if (cmd.type === "FailureCleared") {
            setState("failure", null);
        }
    };
    return { state, dispatchPane, dispatched, setFailure: (f: PaneFailure | null) => setState("failure", f) };
}

const failure = (code: string) => ({ data: { code, title: code }, at: 0, turnAttempted: true }) as unknown as PaneFailure;

describe("useSyntheticAuthRow", () => {
    it("raises the never-started 'Not signed in' row when auth is needed and nothing is showing", () => {
        const pane = fakePane();
        const dispose = createRoot((d) => {
            useSyntheticAuthRow({ canRetry: () => true, paneModel: pane as never });
            return d;
        });
        expect(pane.state.failure?.data.code).toBe("auth");
        expect(pane.dispatched[0]).toMatchObject({ type: "FailureObserved", turnAttempted: false });
        dispose();
    });

    it("retracts its own row once sign-in is no longer needed", () => {
        const pane = fakePane();
        const [canRetry, setCanRetry] = createSignal(true);
        const dispose = createRoot((d) => {
            useSyntheticAuthRow({ canRetry, paneModel: pane as never });
            return d;
        });
        setCanRetry(false);
        expect(pane.state.failure).toBeNull();
        expect(pane.dispatched.at(-1)).toMatchObject({ type: "FailureCleared" });
        dispose();
    });

    it("does nothing while no sign-in is needed", () => {
        const pane = fakePane();
        const dispose = createRoot((d) => {
            useSyntheticAuthRow({ canRetry: () => false, paneModel: pane as never });
            return d;
        });
        expect(pane.dispatched).toEqual([]);
        dispose();
    });
});

describe("useAuthHealth", () => {
    function mount(opts: { failure?: PaneFailure | null; canRetry?: boolean; recheck?: () => Promise<boolean> }) {
        const pane = fakePane(opts.failure ?? null);
        const notifyControllerHealthy = vi.fn();
        const refreshLinkedAccountId = vi.fn(async () => {});
        const handlers: Record<string, () => void> = {};
        const root = createRoot((dispose) => {
            const h = useAuthHealth({
                blockId: "b1",
                agentDefinitionId: () => "agent-1",
                paneModel: pane as never,
                status: {
                    canRetry: () => opts.canRetry ?? false,
                    notifyControllerHealthy,
                    recheckAuthAfterBind: opts.recheck ?? (async () => true),
                },
                refreshLinkedAccountId,
                deps: {
                    failureCode: () => pane.state.failure?.data.code,
                    subscribe: ({ eventType, handler }) => {
                        handlers[eventType] = handler;
                        return () => delete handlers[eventType];
                    },
                    sleep: async () => {},
                },
            });
            return { ...h, dispose };
        });
        return { ...root, pane, notifyControllerHealthy, refreshLinkedAccountId, handlers };
    }

    it("declareAuthHealthy clears an auth failure", () => {
        const m = mount({ failure: failure("auth") });
        m.declareAuthHealthy();
        expect(m.notifyControllerHealthy).toHaveBeenCalled();
        expect(m.pane.state.failure).toBeNull();
        m.dispose();
    });

    it("declareAuthHealthy leaves an unrelated failure alone", () => {
        const m = mount({ failure: failure("rate_limited") });
        m.declareAuthHealthy();
        expect(m.notifyControllerHealthy).toHaveBeenCalled();
        expect(m.pane.state.failure?.data.code).toBe("rate_limited");
        m.dispose();
    });

    it("an identity change refreshes the linked account, and rechecks auth only when blocked", async () => {
        const recheck = vi.fn(async () => true);
        const idle = mount({ recheck });
        idle.handlers["agentidentities:changed:agent-1"]();
        await Promise.resolve();
        expect(idle.refreshLinkedAccountId).toHaveBeenCalled();
        expect(recheck).not.toHaveBeenCalled();
        idle.dispose();

        const blocked = mount({ failure: failure("auth"), recheck });
        blocked.handlers["agentidentities:changed:agent-1"]();
        await new Promise((r) => setTimeout(r, 0));
        expect(recheck).toHaveBeenCalled();
        // A passing recheck is proof of health: the auth row clears.
        expect(blocked.pane.state.failure).toBeNull();
        blocked.dispose();
    });

    it("unsubscribes when disposed", () => {
        const m = mount({});
        expect(Object.keys(m.handlers)).toEqual(["agentidentities:changed:agent-1"]);
        m.dispose();
        expect(Object.keys(m.handlers)).toEqual([]);
    });
});
