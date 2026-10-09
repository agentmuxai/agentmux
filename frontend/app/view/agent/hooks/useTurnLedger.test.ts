// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot } from "solid-js";
import { describe, expect, it, vi } from "vitest";

const handlers: Array<(event: unknown) => void> = [];
vi.mock("@/app/store/mps", () => ({
    muxEventSubscribe: (sub: { handler: (event: unknown) => void }) => {
        handlers.push(sub.handler);
        return () => {};
    },
}));

import type { TurnLedger } from "@/app/store/agent-pane-state/turn-ledger";
import { cachedTurnLedger, taskWakeLine, useTurnLedger } from "./useTurnLedger";

const payload = (turnId: number, passes: number) => ({
    data: { turn_id: turnId, started_at_ms: turnId, active: true, passes },
});

function mount(blockId: string) {
    const dispatched: Array<{ type: string; ledger?: { turnId: number; passes: number } }> = [];
    let dispose = () => {};
    createRoot((d) => {
        dispose = d;
        useTurnLedger(blockId, { dispatchPane: (c: any) => void dispatched.push(c), dispatchDoc: () => {} } as any);
    });
    return { dispatched, dispose };
}

describe("useTurnLedger", () => {
    it("re-feeds the latest ledger to a remounted pane, which gets no replay (#4492)", async () => {
        const first = mount("blk-remount");
        await Promise.resolve();
        handlers.at(-1)!(payload(500, 2));
        expect(first.dispatched.at(-1)?.ledger).toMatchObject({ turnId: 500, passes: 2 });
        first.dispose();

        const second = mount("blk-remount");
        await Promise.resolve();
        expect(second.dispatched[0]?.ledger).toMatchObject({ turnId: 500, passes: 2 });
        second.dispose();
    });

    it("keeps the newest turn when an older one arrives late", async () => {
        const m = mount("blk-order");
        await Promise.resolve();
        handlers.at(-1)!(payload(900, 1));
        handlers.at(-1)!(payload(800, 3));
        expect(cachedTurnLedger("blk-order")?.turnId).toBe(900);
        m.dispose();
    });
});

const now = 1_000_000;
const ledger = (over: Partial<TurnLedger> = {}): TurnLedger => ({
    turnId: 42,
    seq: 1,
    origin: "automated",
    trigger: { kind: "task", from: 'Background command "npm test" completed (exit code 0)' },
    absorbed: [],
    startedAtMs: now - 200,
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

describe("taskWakeLine", () => {
    it("marks a fresh task wake-up in the transcript, keyed by its turn", () => {
        expect(taskWakeLine(ledger(), now)).toEqual({
            type: "ambient_narration",
            id: "turn-trigger-42",
            kind: "turn_trigger",
            text: 'Woke up: Background command "npm test" completed (exit code 0)',
            timestamp: now - 200,
        });
    });

    it("says something even without the task's summary", () => {
        expect(taskWakeLine(ledger({ trigger: { kind: "task", from: null } }), now)?.text).toBe(
            "Woke up: a background task finished",
        );
    });

    it("adds nothing for other turns, later passes, a turn that is over, or an old one replayed on mount", () => {
        expect(taskWakeLine(ledger({ trigger: { kind: "agent", from: "AgentX" } }), now)).toBeNull();
        expect(taskWakeLine(ledger({ passes: 2 }), now)).toBeNull();
        expect(taskWakeLine(ledger({ active: false }), now)).toBeNull();
        expect(taskWakeLine(ledger({ startedAtMs: now - 60_000 }), now)).toBeNull();
    });
});
