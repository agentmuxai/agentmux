// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { UnseenTurnsDot } from "@/app/block/UnseenTurnsDot";
import type { TurnLedger } from "@/app/store/agent-pane-state/turn-ledger";
import { noteTurnLedger, resetTurnAwareness, unseenTurnsFor } from "@/app/store/turn-awareness";
import { TurnAwayDigest } from "./TurnAwayDigest";

const jektTurn = (turnId: number, from: string): TurnLedger => ({
    turnId,
    seq: 1,
    origin: "automated",
    trigger: { kind: "agent", from, external: true },
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

describe("turns you didn't start, seen when you come back", () => {
    beforeEach(() => resetTurnAwareness());
    afterEach(() => cleanup());

    it("the pane's dot shows while they are unseen", () => {
        const { container } = render(() => <UnseenTurnsDot blockId="b1" />);
        expect(container.querySelector(".block-frame-unseen-turns")).toBeNull();
        noteTurnLedger("b1", jektTurn(1, "AgentX"), false);
        const dot = container.querySelector(".block-frame-unseen-turns");
        expect(dot?.getAttribute("aria-label")).toBe("While you were away: 1 turn · 1 jekt (AgentX)");
    });

    it("looking shows the summary once and clears the dot; the next turn retires the summary", () => {
        noteTurnLedger("b1", jektTurn(1, "AgentX"), false);
        noteTurnLedger("b1", jektTurn(2, "Korp"), false);
        const [looking, setLooking] = createSignal(false);
        const [busy, setBusy] = createSignal(false);
        const { container } = render(() => <TurnAwayDigest blockId="b1" looking={looking} busy={busy} />);
        expect(container.querySelector(".agent-away-digest")).toBeNull();

        setLooking(true);
        expect(container.querySelector(".agent-away-digest-text")?.textContent).toBe(
            "While you were away: 2 turns · 2 jekts (AgentX, Korp)",
        );
        expect(unseenTurnsFor("b1")).toBeNull();

        setBusy(true);
        expect(container.querySelector(".agent-away-digest")).toBeNull();
    });

    it("dismiss hides it", () => {
        noteTurnLedger("b1", jektTurn(1, "AgentX"), false);
        const { container } = render(() => <TurnAwayDigest blockId="b1" looking={() => true} busy={() => false} />);
        (container.querySelector(".agent-away-digest-dismiss") as HTMLButtonElement).click();
        expect(container.querySelector(".agent-away-digest")).toBeNull();
    });
});
