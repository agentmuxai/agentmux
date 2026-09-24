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

import { attentionCount, paneEventToNotify } from "./os-notify-bridge";

describe("attentionCount", () => {
    it("counts attention items and tolerates junk", () => {
        expect(attentionCount({ attention: [{}, {}], paused_until_ms: 0 })).toBe(2);
        expect(attentionCount({})).toBe(0);
        expect(attentionCount(undefined)).toBe(0);
        expect(attentionCount({ attention: "nope" })).toBe(0);
    });
});

describe("paneEventToNotify", () => {
    it("maps turn outcomes: completed/errored notify, stopped/interrupted don't", () => {
        expect(paneEventToNotify({ type: "turn-ended", outcome: "completed", statsMerged: false, stoppingCleared: false } as any))
            .toEqual({ event: "turn_completed" });
        expect(paneEventToNotify({ type: "turn-ended", outcome: "errored", statsMerged: false, stoppingCleared: false } as any))
            .toEqual({ event: "turn_errored" });
        expect(paneEventToNotify({ type: "turn-ended", outcome: "stopped", statsMerged: false, stoppingCleared: false } as any)).toBeNull();
        expect(paneEventToNotify({ type: "turn-ended", outcome: "interrupted", statsMerged: false, stoppingCleared: false } as any)).toBeNull();
    });

    it("turn-started resolves a pending 'finished'", () => {
        expect(paneEventToNotify({ type: "turn-started", at: 1 } as any)).toEqual({ event: "turn_started" });
    });

    it("forwards the question text with waiting-for-input", () => {
        expect(paneEventToNotify({ type: "waiting-for-input", question: "Which branch?" })).toEqual({
            event: "input_waiting",
            question: "Which branch?",
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
