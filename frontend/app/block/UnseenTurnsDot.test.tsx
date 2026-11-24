// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { createEffect, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { TurnLedger } from "@/app/store/agent-pane-state/turn-ledger";
import { markTurnsSeen, noteTurnLedger, resetTurnAwareness } from "@/app/store/turn-awareness";
import { UnseenTurnsDot } from "./UnseenTurnsDot";

const jektTurn = (turnId: number): TurnLedger => ({
    turnId,
    seq: 1,
    origin: "automated",
    trigger: { kind: "agent", from: "AgentX", external: true },
    absorbed: [],
    startedAtMs: 1,
    passes: 1,
    active: false,
    inputs: 0,
    countedPasses: 1,
    outputTokens: 1,
    costUsd: 0,
    steps: 1,
    durationApiMs: 0,
    lastPassEndedAtMs: 2,
    settleUntilMs: null,
    endedAtMs: 2,
    end: "completed",
});

describe("UnseenTurnsDot", () => {
    beforeEach(() => resetTurnAwareness());
    afterEach(() => cleanup());

    it("shows what happened while the person was away, and a later turn replaces what it says", () => {
        const { container } = render(() => <UnseenTurnsDot blockId="b1" />);
        noteTurnLedger("b1", jektTurn(1), false);
        const label = () => container.querySelector(".block-frame-unseen-turns")?.getAttribute("aria-label");
        expect(label()).toBe("While you were away: 1 turn · 1 jekt (AgentX)");
        noteTurnLedger("b1", jektTurn(2), false);
        expect(label()).toBe("While you were away: 2 turns · 2 jekts (AgentX)");
    });

    it("goes away without a throw when looking at the pane clears the turns from an effect in the same flush", () => {
        noteTurnLedger("b1", jektTurn(1), false);
        const [looking, setLooking] = createSignal(false);
        const errors: unknown[] = [];
        const onError = (e: ErrorEvent) => errors.push(e.error ?? e.message);
        window.addEventListener("error", onError);
        const { container } = render(() => {
            createEffect(() => {
                if (looking()) markTurnsSeen("b1");
            });
            return <UnseenTurnsDot blockId="b1" />;
        });
        expect(container.querySelector(".block-frame-unseen-turns")).not.toBeNull();
        expect(() => setLooking(true)).not.toThrow();
        expect(container.querySelector(".block-frame-unseen-turns")).toBeNull();
        window.removeEventListener("error", onError);
        expect(errors).toEqual([]);
    });
});
