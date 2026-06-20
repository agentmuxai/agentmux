// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Policy tests for the waiting-ambient-sound subsystem:
 *   • sound-service startWaiting / stopWaiting gating
 *   • reducer endsWithQuestion heuristic (via lastTurnHadQuestion)
 *
 * Spec: SPEC_AGENT_WAITING_AMBIENT_SOUND_2026_06_19.md §4, §6, §9.
 */

import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

// ─── Module mocks ─────────────────────────────────────────────────────

const settings: Record<string, unknown> = {};
const focusState = { focusedBlockId: null as string | null, windowFocused: true };

vi.mock("@/app/store/global", () => ({
    getSettingsKeyAtom: (key: string) => () => settings[key],
}));
vi.mock("@/app/store/focusManager", () => ({
    focusManager: { get blockFocusAtom() { return () => focusState.focusedBlockId; } },
}));
vi.mock("@/app/window/window-focus", () => ({
    makeWindowFocusSignal: () => () => focusState.windowFocused,
}));
vi.mock("solid-js", () => ({
    createRoot: (fn: (d: () => void) => () => void) => fn(() => {}),
    createEffect: () => {},
}));

let captured: ((blockId: string, event: { type: string; [k: string]: unknown }) => void) | null = null;
vi.mock("@/app/store/agent-pane-state-store", () => ({
    addEventListener: (l: typeof captured) => { captured = l; return () => { captured = null; }; },
}));

// ─── Import after mocks ───────────────────────────────────────────────

import {
    __getWaitingTones,
    __resetSoundService,
    installSoundService,
} from "../sound-service";
import { __resetSoundListeners } from "../sound-events";

// ─── Helpers ──────────────────────────────────────────────────────────

function fireEvent(blockId: string, event: { type: string; [k: string]: unknown }): void {
    if (!captured) throw new Error("multicast listener not installed");
    captured(blockId, event);
}

function fireWaitingForInput(blockId: string): void {
    fireEvent(blockId, { type: "waiting-for-input" });
}

function fireWaitingEnded(blockId: string, reason = "submitted"): void {
    fireEvent(blockId, { type: "waiting-ended", reason });
}

// ─── Tests ────────────────────────────────────────────────────────────

describe("waiting-sound-service: startWaiting gating", () => {
    let startSpy: ReturnType<typeof vi.spyOn> | null = null;
    let cleanup: () => void;

    beforeEach(() => {
        vi.useFakeTimers();
        for (const k of Object.keys(settings)) delete settings[k];
        focusState.focusedBlockId = null;
        focusState.windowFocused = true;
        __resetSoundService();
        __resetSoundListeners();
        captured = null;
        cleanup = installSoundService();
    });

    afterEach(() => {
        vi.useRealTimers();
        cleanup();
        startSpy?.mockRestore();
    });

    it("starts a WaitingTonePlayer on waiting-for-input", () => {
        fireWaitingForInput("blk-1");
        expect(__getWaitingTones().has("blk-1")).toBe(true);
    });

    it("master notify:sounds:enabled=false suppresses the waiting tone", () => {
        settings["notify:sounds:enabled"] = false;
        fireWaitingForInput("blk-1");
        expect(__getWaitingTones().has("blk-1")).toBe(false);
    });

    it("notify:sound:agent.waiting.for.input=false suppresses the waiting tone", () => {
        settings["notify:sound:agent.waiting.for.input"] = false;
        fireWaitingForInput("blk-1");
        expect(__getWaitingTones().has("blk-1")).toBe(false);
    });

    it("focus suppression: does not start when pane is focused + window active", () => {
        focusState.focusedBlockId = "blk-1";
        focusState.windowFocused = true;
        fireWaitingForInput("blk-1");
        expect(__getWaitingTones().has("blk-1")).toBe(false);
    });

    it("focus suppression: starts when window is blurred even if pane focused", () => {
        focusState.focusedBlockId = "blk-1";
        focusState.windowFocused = false;
        fireWaitingForInput("blk-1");
        expect(__getWaitingTones().has("blk-1")).toBe(true);
    });

    it("focus suppression: starts for a different pane (not the focused one)", () => {
        focusState.focusedBlockId = "blk-OTHER";
        focusState.windowFocused = true;
        fireWaitingForInput("blk-1");
        expect(__getWaitingTones().has("blk-1")).toBe(true);
    });

    it("suppresswhenfocused=false disables focus suppression", () => {
        settings["notify:sounds:suppresswhenfocused"] = false;
        focusState.focusedBlockId = "blk-1";
        focusState.windowFocused = true;
        fireWaitingForInput("blk-1");
        expect(__getWaitingTones().has("blk-1")).toBe(true);
    });

    it("stopWaiting removes the player from the map", () => {
        fireWaitingForInput("blk-1");
        expect(__getWaitingTones().has("blk-1")).toBe(true);
        fireWaitingEnded("blk-1");
        expect(__getWaitingTones().has("blk-1")).toBe(false);
    });

    it("waiting-ended with reason=typing also removes the player", () => {
        fireWaitingForInput("blk-1");
        fireWaitingEnded("blk-1", "typing");
        expect(__getWaitingTones().has("blk-1")).toBe(false);
    });

    it("waiting-ended with reason=closed also removes the player", () => {
        fireWaitingForInput("blk-1");
        fireWaitingEnded("blk-1", "closed");
        expect(__getWaitingTones().has("blk-1")).toBe(false);
    });

    it("multiple panes each get independent players", () => {
        fireWaitingForInput("blk-A");
        fireWaitingForInput("blk-B");
        expect(__getWaitingTones().has("blk-A")).toBe(true);
        expect(__getWaitingTones().has("blk-B")).toBe(true);
        fireWaitingEnded("blk-A");
        expect(__getWaitingTones().has("blk-A")).toBe(false);
        expect(__getWaitingTones().has("blk-B")).toBe(true);
    });

    it("5-minute auto-stop clears the player", () => {
        fireWaitingForInput("blk-1");
        expect(__getWaitingTones().has("blk-1")).toBe(true);
        vi.advanceTimersByTime(5 * 60 * 1000 + 100);
        expect(__getWaitingTones().has("blk-1")).toBe(false);
    });
});

// ─── Reducer endsWithQuestion heuristic ───────────────────────────────

describe("endsWithQuestion heuristic (via reducer)", () => {
    // We test endsWithQuestion indirectly by importing the reducer and
    // checking the state it produces, so no mocks needed beyond imports.
    let update: typeof import("@/app/store/agent-pane-state/reducer").update;
    let initialState: typeof import("@/app/store/agent-pane-state/types").initialState;

    beforeAll(async () => {
        ({ update } = await import("@/app/store/agent-pane-state/reducer"));
        ({ initialState } = await import("@/app/store/agent-pane-state/types"));
    });

    function completedTurn(lastAssistantText?: string) {
        const state = initialState("agent-1");
        // Fake a streaming state so TurnEnd is valid
        const streaming = { ...state, turnPhase: { kind: "Streaming", bufferSize: 0, toolsActive: 0, lastEventMs: 0 } } as typeof state;
        return update(streaming, { type: "TurnEnd", stats: null, lastAssistantText });
    }

    it("sets lastTurnHadQuestion=true when text ends with ?", () => {
        expect(completedTurn("What would you like to do?").state.lastTurnHadQuestion).toBe(true);
    });

    it("sets lastTurnHadQuestion=false when text does not end with ?", () => {
        expect(completedTurn("I've finished the task.").state.lastTurnHadQuestion).toBe(false);
    });

    it("sets lastTurnHadQuestion=false when lastAssistantText is absent", () => {
        expect(completedTurn(undefined).state.lastTurnHadQuestion).toBe(false);
    });

    it("handles trailing quotes before the question mark", () => {
        expect(completedTurn("Is this correct?\"").state.lastTurnHadQuestion).toBe(true);
    });

    it("handles trailing closing parens before the question mark", () => {
        expect(completedTurn("Ready to proceed?)").state.lastTurnHadQuestion).toBe(true);
    });

    it("sets lastTurnHadQuestion=false for non-completed outcome even with ?", () => {
        const state = initialState("agent-1");
        const interrupting = { ...state, turnPhase: { kind: "Interrupting", reason: "user", sigintSentAt: 0 } } as typeof state;
        const result = update(interrupting, { type: "TurnEnd", stats: null, lastAssistantText: "Do you want me to stop?" });
        // outcome is "stopped" → flag should be false
        expect(result.state.lastTurnHadQuestion).toBe(false);
    });

    it("TurnReset clears lastTurnHadQuestion", () => {
        const withQuestion = completedTurn("Should I continue?").state;
        expect(withQuestion.lastTurnHadQuestion).toBe(true);
        const reset = update(withQuestion, { type: "TurnReset" });
        expect(reset.state.lastTurnHadQuestion).toBe(false);
    });
});
