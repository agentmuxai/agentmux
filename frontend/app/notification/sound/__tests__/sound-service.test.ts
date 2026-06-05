// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Policy tests for the sound service — gating by settings, coalesce
 * window, focus suppression, and replay mode. The player is not
 * primed (no AudioContext in jsdom), so we observe `play()` calls
 * on the player instead of measuring audio output.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// ─── Module mocks ─────────────────────────────────────────────────────

const settings: Record<string, unknown> = {};
function setSetting(key: string, value: unknown): void {
    settings[key] = value;
}
function resetSettings(): void {
    for (const k of Object.keys(settings)) delete settings[k];
}

const focusState = { focusedBlockId: null as string | null, windowFocused: true };

vi.mock("@/app/store/global", () => ({
    getSettingsKeyAtom: (key: string) => () => settings[key],
}));

vi.mock("@/app/store/focusManager", () => ({
    focusManager: {
        get blockFocusAtom() {
            return () => focusState.focusedBlockId;
        },
    },
}));

vi.mock("@/app/window/window-focus", () => ({
    makeWindowFocusSignal: () => () => focusState.windowFocused,
}));

// Capture the multicast listener so the test can drive reducer events
// without registering a full pane. The sound service calls
// `addEventListener` once during `installSoundService()`.
let captured:
    | ((blockId: string, event: { type: string; [k: string]: unknown }) => void)
    | null = null;
vi.mock("@/app/store/agent-pane-state-store", () => ({
    addEventListener: (
        l: (blockId: string, event: { type: string; [k: string]: unknown }) => void,
    ) => {
        captured = l;
        return () => {
            captured = null;
        };
    },
}));

// ─── Import after mocks ───────────────────────────────────────────────

import {
    __getSoundPlayer,
    __resetSoundService,
    installSoundService,
    setReplayMode,
} from "../sound-service";
import { __resetSoundListeners } from "../sound-events";

describe("sound-service policy", () => {
    let playSpy: ReturnType<typeof vi.spyOn>;
    let cleanup: () => void;

    beforeEach(() => {
        resetSettings();
        focusState.focusedBlockId = null;
        focusState.windowFocused = true;
        __resetSoundService();
        __resetSoundListeners();
        captured = null;
        playSpy = vi.spyOn(__getSoundPlayer(), "play").mockImplementation(() => {});
        cleanup = installSoundService();
    });

    afterEach(() => {
        cleanup();
        playSpy.mockRestore();
        setReplayMode(false);
    });

    function fireTurnEnded(blockId: string, outcome: string): void {
        if (!captured) throw new Error("multicast listener was never installed");
        captured(blockId, {
            type: "turn-ended",
            outcome,
            statsMerged: false,
            stoppingCleared: false,
        });
    }

    it("plays the complete sound on outcome=completed", () => {
        fireTurnEnded("blk-1", "completed");
        expect(playSpy).toHaveBeenCalledTimes(1);
        expect((playSpy.mock.calls[0][0] as { id: string }).id).toBe("agent.turn.complete");
    });

    it("plays the error sound on outcome=errored", () => {
        fireTurnEnded("blk-1", "errored");
        expect(playSpy).toHaveBeenCalledTimes(1);
        expect((playSpy.mock.calls[0][0] as { id: string }).id).toBe("agent.turn.error");
    });

    it("plays the interrupted sound on outcome=stopped", () => {
        fireTurnEnded("blk-1", "stopped");
        expect(playSpy).toHaveBeenCalledTimes(1);
        expect((playSpy.mock.calls[0][0] as { id: string }).id).toBe(
            "agent.turn.interrupted",
        );
    });

    it("plays the interrupted sound on outcome=interrupted", () => {
        fireTurnEnded("blk-1", "interrupted");
        expect(playSpy).toHaveBeenCalledTimes(1);
        expect((playSpy.mock.calls[0][0] as { id: string }).id).toBe(
            "agent.turn.interrupted",
        );
    });

    it("master notify:sounds:enabled=false suppresses all sounds", () => {
        setSetting("notify:sounds:enabled", false);
        fireTurnEnded("blk-1", "completed");
        expect(playSpy).not.toHaveBeenCalled();
    });

    it("per-event setting=false suppresses just that sound", () => {
        setSetting("notify:sound:agent.turn.complete", false);
        fireTurnEnded("blk-1", "completed");
        fireTurnEnded("blk-1", "errored");
        expect(playSpy).toHaveBeenCalledTimes(1);
        expect((playSpy.mock.calls[0][0] as { id: string }).id).toBe("agent.turn.error");
    });

    it("coalesces a second fire within the coalesce window", () => {
        fireTurnEnded("blk-1", "completed");
        fireTurnEnded("blk-1", "completed"); // within 300ms
        expect(playSpy).toHaveBeenCalledTimes(1);
    });

    it("focus-suppression drops sound when source pane is focused AND window is focused", () => {
        focusState.focusedBlockId = "blk-1";
        focusState.windowFocused = true;
        fireTurnEnded("blk-1", "completed");
        expect(playSpy).not.toHaveBeenCalled();
    });

    it("focus-suppression does NOT drop sound when the window is blurred", () => {
        focusState.focusedBlockId = "blk-1";
        focusState.windowFocused = false;
        fireTurnEnded("blk-1", "completed");
        expect(playSpy).toHaveBeenCalledTimes(1);
    });

    it("focus-suppression does NOT drop sound when a different pane is focused", () => {
        focusState.focusedBlockId = "blk-OTHER";
        focusState.windowFocused = true;
        fireTurnEnded("blk-1", "completed");
        expect(playSpy).toHaveBeenCalledTimes(1);
    });

    it("focus-suppression can be disabled via setting", () => {
        focusState.focusedBlockId = "blk-1";
        focusState.windowFocused = true;
        setSetting("notify:sounds:suppresswhenfocused", false);
        fireTurnEnded("blk-1", "completed");
        expect(playSpy).toHaveBeenCalledTimes(1);
    });

    it("replay mode drops every play", () => {
        setReplayMode(true);
        fireTurnEnded("blk-1", "completed");
        expect(playSpy).not.toHaveBeenCalled();
    });

    it("stream-stalled event maps to the stalled sound", () => {
        if (!captured) throw new Error("listener not installed");
        captured("blk-1", { type: "stream-stalled", at: 100 });
        expect(playSpy).toHaveBeenCalledTimes(1);
        expect((playSpy.mock.calls[0][0] as { id: string }).id).toBe("agent.stream.stalled");
    });

    it("pending-accepted only fires when wasPresent=true", () => {
        if (!captured) throw new Error("listener not installed");
        captured("blk-1", { type: "pending-accepted", id: "m1", wasPresent: false });
        expect(playSpy).not.toHaveBeenCalled();
        captured("blk-1", { type: "pending-accepted", id: "m1", wasPresent: true });
        expect(playSpy).toHaveBeenCalledTimes(1);
        expect((playSpy.mock.calls[0][0] as { id: string }).id).toBe("agent.message.accepted");
    });
});
