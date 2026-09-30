// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot } from "solid-js";
import { describe, expect, it, vi } from "vitest";
import { createTurnConfirmation } from "./turn-confirmation";

function mount() {
    const reconcile = vi.fn();
    const seen: Array<{ idle: boolean; active: boolean }> = [];
    return createRoot((dispose) => {
        const tc = createTurnConfirmation({
            reconcile,
            // Records what the flush's own safety gate would read at call time.
            onTurnEnded: () => seen.push({ idle: tc.isBackendTurnConfirmedIdle(), active: tc.isBackendTurnActive() }),
        });
        return { tc, reconcile, seen, dispose };
    });
}

describe("createTurnConfirmation", () => {
    it("before any live reading, the turn is neither active nor confirmed idle", () => {
        const { tc, dispose } = mount();
        expect(tc.isBackendTurnActive()).toBe(false);
        expect(tc.isBackendTurnConfirmedIdle()).toBe(false);
        dispose();
    });

    it("counts one turn end per true -> false edge", () => {
        const { tc, dispose } = mount();
        tc.trackTurnJustEnded(true);
        tc.trackTurnJustEnded(true);
        expect(tc.turnJustEndedAtom()).toBe(0);
        tc.trackTurnJustEnded(false);
        tc.trackTurnJustEnded(false);
        expect(tc.turnJustEndedAtom()).toBe(1);
        dispose();
    });

    // Codex P1 on #2338: the state must update BEFORE the turn-end callback,
    // whose deferred refresh checks isBackendTurnConfirmedIdle() synchronously.
    it("updates the state before calling onTurnEnded, so the callback sees confirmed idle", () => {
        const { tc, seen, dispose } = mount();
        tc.trackTurnJustEnded(true);
        tc.trackTurnJustEnded(false);
        expect(seen).toEqual([{ idle: true, active: false }]);
        dispose();
    });

    // reagent P1 on #2241: only live readings drive the edge; the reconcile
    // path (also used by the late mount-time one-shot) must not.
    it("reconcileTurnActive dispatches but never moves the edge detector", () => {
        const { tc, reconcile, seen, dispose } = mount();
        tc.reconcileTurnActive(true);
        tc.reconcileTurnActive(false);
        expect(reconcile.mock.calls).toEqual([[true], [false]]);
        expect(tc.turnJustEndedAtom()).toBe(0);
        expect(seen).toEqual([]);
        expect(tc.isBackendTurnConfirmedIdle()).toBe(false);
        dispose();
    });
});
