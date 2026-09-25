// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * `paneEventToNotify` — the renderer's half of the notification contract
 * (SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md §3). The subtle rule pinned
 * here: `waiting-ended` with reason "closed" must NOT resolve the toast —
 * a pane unmounting (window closed into background mode) while the agent is
 * still blocked is exactly when the user needs the notification.
 */

import { describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/agent-pane-state-store", () => ({ addEventListener: () => () => {} }));
vi.mock("@/app/store/focusManager", () => ({ focusManager: { blockFocusAtom: () => null } }));
vi.mock("@/app/store/global", () => ({
    getApi: () => ({}),
    getSettingsKeyAtom: () => () => undefined,
    MOS: {},
    setActiveTab: async () => {},
    workspace: () => null,
}));
vi.mock("@/app/store/mps", () => ({ muxEventSubscribe: () => () => {} }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/window/window-focus", () => ({ makeWindowFocusSignal: () => () => true }));
vi.mock("@/layout/lib/layoutModelHooks", () => ({ getLayoutModelForTabById: () => undefined }));

import { attentionCount, inputWaitingCount, paneEventToNotify } from "./os-notify-bridge";

describe("attentionCount", () => {
    it("counts attention items and tolerates junk", () => {
        expect(attentionCount({ attention: [{}, {}], paused_until_ms: 0 })).toBe(2);
        expect(attentionCount({})).toBe(0);
        expect(attentionCount(undefined)).toBe(0);
        expect(attentionCount({ attention: "nope" })).toBe(0);
    });

    it("counts only input_waiting for the flash signal", () => {
        const data = { attention: [{ kind: "input_waiting" }, { kind: "agent_crashed" }, { kind: "message_needs_review" }] };
        expect(attentionCount(data)).toBe(3);
        expect(inputWaitingCount(data)).toBe(1);
        expect(inputWaitingCount({})).toBe(0);
    });
});

describe("paneEventToNotify", () => {
    it("never reports turn start/finish (srv's controller status owns that)", () => {
        const ended = (outcome: string) => ({ type: "turn-ended", outcome, statsMerged: false, stoppingCleared: false }) as any;
        expect(paneEventToNotify(ended("completed"))).toBeNull();
        expect(paneEventToNotify(ended("errored"))).toBeNull();
        expect(paneEventToNotify({ type: "turn-started", at: 1 } as any)).toBeNull();
    });

    it("reports the user's own stop so that turn-end isn't announced", () => {
        const ended = (outcome: string) => ({ type: "turn-ended", outcome, statsMerged: false, stoppingCleared: false }) as any;
        expect(paneEventToNotify(ended("stopped"))).toEqual({ event: "turn_stopped" });
        expect(paneEventToNotify(ended("interrupted"))).toEqual({ event: "turn_stopped" });
    });

    it("forwards the question text with waiting-for-input", () => {
        expect(paneEventToNotify({ type: "waiting-for-input", question: "Which branch?" })).toEqual({
            event: "input_waiting",
            question: "Which branch?",
        });
    });

    it("forwards how many questions the call asks", () => {
        expect(paneEventToNotify({ type: "waiting-for-input", question: "Which branch?", questionCount: 3 })).toEqual({
            event: "input_waiting",
            question: "Which branch?",
            question_count: 3,
        });
    });

    it("resolves on submitted, but NOT on closed", () => {
        expect(paneEventToNotify({ type: "waiting-ended", reason: "submitted" } as any)).toEqual({ event: "input_resolved" });
        expect(paneEventToNotify({ type: "waiting-ended", reason: "closed" } as any)).toBeNull();
    });

    it("ignores unrelated events", () => {
        expect(paneEventToNotify({ type: "pending-accepted" } as any)).toBeNull();
    });
});
