// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Policy tests for the waiting-ambient-sound subsystem:
 *   • sound-service startWaiting / stopWaiting gating
 *
 * The tone is now triggered by AskUserQuestion panel visibility
 * (agent-view fires waiting-for-input / waiting-ended via fireEvent
 * when pendingQuestions changes), not by the text-ends-with-? heuristic.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

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

const flashes: string[] = [];
vi.mock("@/app/notification/activity-flash", () => ({
    emitActivityFlash: (t: { blockId: string }) => flashes.push(t.blockId),
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

    it("focus suppression: registers player but does not start when pane is focused + window active", () => {
        focusState.focusedBlockId = "blk-1";
        focusState.windowFocused = true;
        fireWaitingForInput("blk-1");
        // Player is registered so focus-leave effect can resume it (spec §8).
        const wp = __getWaitingTones().get("blk-1");
        expect(wp).toBeDefined();
        expect(wp?.__isRunning()).toBe(false);
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

    it("flashes the pane's tab with each loop of the tone, and stops with it", () => {
        flashes.length = 0;
        fireWaitingForInput("blk-1");
        expect(flashes).toEqual(["blk-1"]);
        vi.advanceTimersByTime(2500);
        expect(flashes).toEqual(["blk-1", "blk-1"]);
        fireWaitingEnded("blk-1");
        vi.advanceTimersByTime(5000);
        expect(flashes).toHaveLength(2);
    });

    it("a request with no pane is quiet while its window is in front, and has no tab to flash", () => {
        flashes.length = 0;
        focusState.windowFocused = true;
        fireWaitingForInput("app:k1");
        expect(__getWaitingTones().get("app:k1")?.__isRunning()).toBe(false);
        expect(flashes).toHaveLength(0);
        __resetSoundService();
        __resetSoundListeners();
        cleanup();
        cleanup = installSoundService();
        focusState.windowFocused = false;
        fireWaitingForInput("app:k2");
        expect(__getWaitingTones().has("app:k2")).toBe(true);
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

