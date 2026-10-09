// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it } from "vitest";
import type { TurnLedger, TurnTrigger } from "./agent-pane-state/turn-ledger";
import { awaySummary, installTurnAwareness, markTurnsSeen, noteTurnLedger, resetTurnAwareness, unseenTurnsFor } from "./turn-awareness";

const ended = (turnId: number, trigger: TurnTrigger | null, end: "completed" | "exited" | null = "completed"): TurnLedger => ({
    turnId,
    seq: 1,
    origin: "automated",
    trigger,
    absorbed: [],
    startedAtMs: 1_000,
    passes: 1,
    active: false,
    inputs: 0,
    countedPasses: 1,
    outputTokens: 10,
    costUsd: 0,
    steps: 1,
    durationApiMs: 0,
    lastPassEndedAtMs: 2_000,
    settleUntilMs: null,
    endedAtMs: end ? 2_000 : null,
    end,
});
const jekt = (from: string): TurnTrigger => ({ kind: "agent", from });

describe("turn awareness", () => {
    beforeEach(() => resetTurnAwareness());

    it("counts an external turn that finished while you weren't looking, once", () => {
        expect(noteTurnLedger("b1", ended(1, jekt("AgentX")), false)).toBe(true);
        expect(noteTurnLedger("b1", ended(1, jekt("AgentX")), false)).toBe(false); // published twice
        expect(unseenTurnsFor("b1")).toMatchObject({ count: 1, triggers: [jekt("AgentX")] });
    });

    it("ignores your own turns, unfinished ones, crashes, and ones you watched", () => {
        expect(noteTurnLedger("b1", ended(1, { kind: "user", from: null }), false)).toBe(false);
        expect(noteTurnLedger("b1", ended(2, { kind: "broadcast", from: null }), false)).toBe(false);
        expect(noteTurnLedger("b1", ended(3, jekt("A"), null), false)).toBe(false);
        expect(noteTurnLedger("b1", ended(4, jekt("A"), "exited"), false)).toBe(false);
        expect(noteTurnLedger("b1", ended(5, jekt("A")), true)).toBe(false);
        expect(unseenTurnsFor("b1")).toBeNull();
    });

    it("looking clears them, and returns what there was", () => {
        noteTurnLedger("b1", ended(1, jekt("A")), false);
        noteTurnLedger("b2", ended(2, jekt("B")), false);
        expect(markTurnsSeen("b1")?.count).toBe(1);
        expect(unseenTurnsFor("b1")).toBeNull();
        expect(unseenTurnsFor("b2")?.count).toBe(1);
        expect(markTurnsSeen("b1")).toBeNull();
    });

    it("summarises by kind, naming senders", () => {
        noteTurnLedger("b", ended(1, jekt("AgentX")), false);
        noteTurnLedger("b", ended(2, jekt("Korp")), false);
        noteTurnLedger("b", ended(3, { kind: "task", from: 'Background command "npm test" completed' }), false);
        noteTurnLedger("b", ended(4, { kind: "service", from: "github-consumer" }), false);
        expect(awaySummary(unseenTurnsFor("b")!)).toBe(
            "While you were away: 4 turns · 2 jekts (AgentX, Korp) · 1 finished task · 1 notice (github-consumer)",
        );
    });

    it("installs one app-wide watcher, fed every block's ledger", () => {
        let feed: ((blockId: string, data: unknown) => void) | null = null;
        const dispose = installTurnAwareness({
            subscribe: (h) => {
                feed = h;
                return () => {};
            },
            watching: (b) => b === "focused",
        });
        const data = { turn_id: 9, started_at_ms: 1, active: false, end: "completed", ended_at_ms: 5, trigger: { kind: "schedule", from: "cron" } };
        feed!("background", data);
        feed!("focused", { ...data, turn_id: 10 });
        expect(unseenTurnsFor("background")?.count).toBe(1);
        expect(unseenTurnsFor("focused")).toBeNull();
        dispose();
    });
});
