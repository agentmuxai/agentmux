// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §4.4: the pane's reading
// of srv's turn ledger.

import { describe, expect, it } from "vitest";
import { update } from "./reducer";
import { initialState } from "./types";
import {
    HELD_FLUSH_JOIN_MS,
    isNewerLedger,
    parseTurnLedger,
    turnEndedAt,
    turnLiveOutput,
    turnOpen,
    turnSettling,
    turnToJoin,
    type TurnLedger,
} from "./turn-ledger";

const ledger = (over: Partial<TurnLedger> = {}): TurnLedger => ({
    turnId: 1_000,
    seq: 1,
    origin: "user",
    startedAtMs: 1_000,
    passes: 1,
    active: true,
    inputs: 0,
    countedPasses: 0,
    outputTokens: 0,
    costUsd: 0,
    steps: 0,
    durationApiMs: 0,
    lastPassEndedAtMs: null,
    settleUntilMs: null,
    endedAtMs: null,
    end: null,
    ...over,
});

describe("parseTurnLedger", () => {
    it("reads srv's agentturn payload", () => {
        const l = parseTurnLedger({
            block_id: "b",
            turn_id: 1_791_400_000_000,
            origin: "automated",
            started_at_ms: 1_791_400_000_000,
            passes: 2,
            active: false,
            inputs: 1,
            counted_passes: 2,
            output_tokens: 4_200,
            cost_usd: 0.41,
            steps: 9,
            duration_api_ms: 60_000,
            last_pass_ended_at_ms: 1_791_400_090_000,
            settle_until_ms: 1_791_400_092_000,
        });
        expect(l).toMatchObject({
            origin: "automated",
            passes: 2,
            active: false,
            countedPasses: 2,
            outputTokens: 4_200,
            settleUntilMs: 1_791_400_092_000,
            endedAtMs: null,
            end: null,
        });
    });

    it("rejects anything that isn't one", () => {
        expect(parseTurnLedger(null)).toBeNull();
        expect(parseTurnLedger({ turn_id: 1 })).toBeNull();
        expect(parseTurnLedger({ turn_id: 1, started_at_ms: 1, active: "yes" })).toBeNull();
    });

    it("leaves an unknown origin unknown", () => {
        expect(parseTurnLedger({ turn_id: 1, started_at_ms: 1, active: true, origin: "cron" })?.origin).toBeNull();
    });
});

describe("turn state over time", () => {
    const settling = ledger({ active: false, lastPassEndedAtMs: 5_000, settleUntilMs: 7_000 });

    it("a running pass is an open turn", () => {
        expect(turnOpen(ledger(), 9_999_999)).toBe(true);
        expect(turnSettling(ledger(), 0)).toBe(false);
    });

    it("between passes, the turn is open until its settle window lapses", () => {
        expect(turnSettling(settling, 6_000)).toBe(true);
        expect(turnOpen(settling, 7_000)).toBe(true);
        expect(turnOpen(settling, 7_001)).toBe(false);
    });

    it("a lapsed turn ended at its last pass; an ended one at its end", () => {
        expect(turnEndedAt(settling, 6_000)).toBeNull();
        expect(turnEndedAt(settling, 8_000)).toBe(5_000);
        expect(turnEndedAt(ledger({ active: false, endedAtMs: 4_000, end: "completed" }), 0)).toBe(4_000);
    });
});

describe("turnLiveOutput", () => {
    const tokens = (output: number, stamp?: { turnId: number; pass: number }) => ({
        input: 1_000,
        output,
        ...(stamp ? { ledgerTurnId: stamp.turnId, ledgerPass: stamp.pass } : {}),
    });

    it("without a ledger, is the pass's own count (as before)", () => {
        expect(turnLiveOutput(null, tokens(300), 0)).toBe(300);
        expect(turnLiveOutput(null, null, 0)).toBeUndefined();
    });

    it("in a turn's first pass, is that pass's count", () => {
        expect(turnLiveOutput(ledger(), tokens(300, { turnId: 1_000, pass: 1 }), 0)).toBe(300);
    });

    it("adds the running pass to the passes srv has counted", () => {
        const l = ledger({ passes: 2, countedPasses: 1, outputTokens: 2_000 });
        expect(turnLiveOutput(l, tokens(300, { turnId: 1_000, pass: 2 }), 0)).toBe(2_300);
    });

    it("never counts a pass twice: tokens srv has already counted are left out", () => {
        // srv counted pass 1 (its result) before the pane cleared its tokens.
        const l = ledger({ active: false, countedPasses: 1, outputTokens: 2_000, settleUntilMs: 10_000 });
        expect(turnLiveOutput(l, tokens(1_900, { turnId: 1_000, pass: 1 }), 0)).toBe(2_000);
    });

    it("ignores tokens from another turn", () => {
        const l = ledger({ turnId: 2_000, passes: 2, countedPasses: 1, outputTokens: 500 });
        expect(turnLiveOutput(l, tokens(1_900, { turnId: 1_000, pass: 2 }), 0)).toBe(500);
    });

    it("after the turn ended, is the pass's own count again", () => {
        const l = ledger({ active: false, countedPasses: 2, outputTokens: 4_000, endedAtMs: 10, end: "completed" });
        expect(turnLiveOutput(l, tokens(50), 100)).toBe(50);
    });
});

describe("reducer: TurnObserved and the pass stamp", () => {
    it("stores the ledger, and ignores an older turn's arriving late", () => {
        let s = initialState("a");
        s = update(s, { type: "TurnObserved", ledger: ledger({ turnId: 2_000 }) }).state;
        s = update(s, { type: "TurnObserved", ledger: ledger({ turnId: 1_000 }) }).state;
        expect(s.turnLedger?.turnId).toBe(2_000);
        s = update(s, { type: "TurnObserved", ledger: ledger({ turnId: 2_000, passes: 2 }) }).state;
        expect(s.turnLedger?.passes).toBe(2);
    });

    it("stamps a call's tokens with the turn and pass it began in, through the call", () => {
        let s = initialState("a");
        s = update(s, { type: "TurnObserved", ledger: ledger({ turnId: 3_000, passes: 2 }) }).state;
        s = update(s, { type: "TokensIn", input: 900 }).state;
        expect(s.turnTokens).toMatchObject({ ledgerTurnId: 3_000, ledgerPass: 2 });
        s = update(s, { type: "TokensOut", output: 120 }).state;
        expect(s.turnTokens).toMatchObject({ ledgerTurnId: 3_000, ledgerPass: 2, output: 120 });
    });

    it("leaves tokens unstamped when srv has reported no turn", () => {
        const s = update(initialState("a"), { type: "TokensIn", input: 900 }).state;
        expect(s.turnTokens?.ledgerTurnId).toBeUndefined();
    });
});

describe("turnToJoin", () => {
    it("is the open turn, or the one that only just ended", () => {
        expect(turnToJoin(ledger(), 0)).toBe(1_000);
        const ended = ledger({ active: false, lastPassEndedAtMs: 5_000, endedAtMs: 5_000, end: "completed" });
        expect(turnToJoin(ended, 5_000 + HELD_FLUSH_JOIN_MS)).toBe(1_000);
        expect(turnToJoin(ended, 5_001 + HELD_FLUSH_JOIN_MS)).toBeUndefined();
        expect(turnToJoin(null, 0)).toBeUndefined();
    });
});

describe("isNewerLedger", () => {
    it("takes a later turn, or a later state of the same turn, never an older one landing late", () => {
        const cur = ledger({ turnId: 5, seq: 10 });
        expect(isNewerLedger(ledger({ turnId: 5, seq: 11 }), cur)).toBe(true);
        expect(isNewerLedger(ledger({ turnId: 5, seq: 9 }), cur)).toBe(false);
        expect(isNewerLedger(ledger({ turnId: 6, seq: 2 }), cur)).toBe(true);
        expect(isNewerLedger(ledger({ turnId: 4, seq: 99 }), cur)).toBe(false);
        expect(isNewerLedger(cur, null)).toBe(true);
    });

    it("the reducer drops an older state of the same turn", () => {
        let s = initialState("a");
        s = update(s, { type: "TurnObserved", ledger: ledger({ seq: 5, active: true }) }).state;
        s = update(s, { type: "TurnObserved", ledger: ledger({ seq: 4, active: false }) }).state;
        expect(s.turnLedger?.active).toBe(true);
    });
});
