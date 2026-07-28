// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Pins the `onTurnStartFromQueue` wiring added alongside the scroll-follow
 * hardening pass (reagent P2: the queue-drain TurnStart path previously had
 * no test coverage). A queued message that the backend auto-accepts while
 * the pane is idle must both dispatch TurnStart AND re-engage message-list
 * auto-scroll; a queued message accepted mid-turn (steering) must dispatch
 * neither, since TurnStart was already fired by the composer send. See
 * docs/specs/SPEC_WORKING_STATE_AND_SCROLL_FOLLOW_HARDENING_2026_07_27.md §3.
 */

import { createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    handlers: new Map<string, (e: unknown) => void>(),
    turnPhase: { kind: "Done", outcome: "completed", finishedAt: 0 } as { kind: string; [k: string]: unknown },
}));

vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: vi.fn((sub: { eventType: string; handler: (e: unknown) => void }) => {
        hub.handlers.set(sub.eventType, sub.handler);
        return () => hub.handlers.delete(sub.eventType);
    }),
}));
vi.mock("@/app/store/wos", () => ({ makeORef: (a: string, b: string) => `${a}:${b}` }));
vi.mock("@/app/store/agent-pane-state-store", () => ({
    snapshot: (_blockId: string) => ({ turnPhase: hub.turnPhase }),
}));
vi.mock("@/log/render-trail", () => ({ trail: vi.fn() }));

import { usePendingMessageAcceptance } from "./usePendingMessageAcceptance";
import type { PendingMessage } from "../state";

const BLOCK_ID = "b";

const fire = (data: unknown) => {
    const h = hub.handlers.get("agent-message-accepted");
    if (!h) throw new Error("agent-message-accepted handler not registered — onMount did not run");
    h({ data });
};

function setup(pending: PendingMessage[]) {
    const dispatched: unknown[] = [];
    const pushedNodes: unknown[] = [];
    const onTurnStartFromQueue = vi.fn();
    let dispose: () => void = () => {};
    createRoot((d) => {
        dispose = d;
        usePendingMessageAcceptance({
            blockId: BLOCK_ID,
            model: { dispatchPane: (cmd: unknown) => dispatched.push(cmd) } as any,
            pendingMessagesAtom: [() => pending, () => {}] as any,
            queue: {
                pushNewNode: (n: unknown) => pushedNodes.push(n),
                scheduleFlush: vi.fn(),
            } as any,
            hasNodeId: () => false,
            addNodeId: () => {},
            onTurnStartFromQueue,
        });
    });
    return { dispatched, pushedNodes, onTurnStartFromQueue, dispose };
}

beforeEach(() => {
    hub.handlers.clear();
});
afterEach(() => {
    vi.clearAllMocks();
});

describe("usePendingMessageAcceptance — onTurnStartFromQueue", () => {
    it("dispatches TurnStart and re-engages scroll-follow when accepted while idle (Done)", () => {
        hub.turnPhase = { kind: "Done", outcome: "completed", finishedAt: 0 };
        const pending: PendingMessage[] = [{ id: "m1", text: "hello" } as PendingMessage];
        const { dispatched, onTurnStartFromQueue, dispose } = setup(pending);

        fire({ message_id: "m1" });

        expect(dispatched).toContainEqual({ type: "PendingMessageAccepted", id: "m1" });
        expect(dispatched.some((c: any) => c.type === "TurnStart")).toBe(true);
        expect(onTurnStartFromQueue).toHaveBeenCalledTimes(1);
        dispose();
    });

    it("does NOT dispatch TurnStart or re-engage scroll-follow when accepted mid-turn (Streaming)", () => {
        hub.turnPhase = { kind: "Streaming", toolsActive: 0, bufferSize: 0, lastEventMs: 0 };
        const pending: PendingMessage[] = [{ id: "m2", text: "steer" } as PendingMessage];
        const { dispatched, onTurnStartFromQueue, dispose } = setup(pending);

        fire({ message_id: "m2" });

        expect(dispatched.some((c: any) => c.type === "TurnStart")).toBe(false);
        expect(onTurnStartFromQueue).not.toHaveBeenCalled();
        dispose();
    });

    it("ignores an accepted event for an unknown message id", () => {
        hub.turnPhase = { kind: "Done", outcome: "completed", finishedAt: 0 };
        const { dispatched, onTurnStartFromQueue, dispose } = setup([]);

        fire({ message_id: "does-not-exist" });

        expect(dispatched).toEqual([]);
        expect(onTurnStartFromQueue).not.toHaveBeenCalled();
        dispose();
    });
});
