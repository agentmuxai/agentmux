// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Regression test for PR #2751 review (re-review round): getVoiceSession()
 * must return a STABLE object so long-lived callers that cache it once at
 * component setup (MicButton.tsx, AgentFooter.tsx: `const voice =
 * getVoiceSession()` at the top of the component body) still observe an
 * engine change made later via Settings -> Recording's live engine picker.
 * An earlier fix in this same PR rebuilt the underlying session correctly
 * but returned it directly, so only call sites that re-invoke
 * getVoiceSession() fresh on every use (keymodel.ts's global hotkey) actually
 * saw the rebuild — this test exercises the cached-at-setup pattern instead,
 * which is the primary per-pane mic-button path.
 */

import { createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSignalAtom } from "@/util/util";

let engineSignal: ReturnType<typeof createSignal<string | null>>;

vi.mock("@/app/store/global", () => ({
    getSettingsKeyAtom: (key: string) => {
        if (key === "voice:engine") return engineSignal[0];
        return () => null;
    },
}));

let nextMockSessionId = 0;
vi.mock("./whisperVoiceEngine", () => ({
    createWhisperVoiceSession: vi.fn(() => {
        const id = ++nextMockSessionId;
        const isListening = createSignalAtom(false);
        const currentTargetId = createSignalAtom<string | null>(null);
        const lastError = createSignalAtom<string | null>(null);
        return {
            __id: id,
            isListening,
            currentTargetId,
            lastError,
            isAvailable: () => true,
            toggleListening: () => isListening._set(!isListening()),
            registerPane: (blockId: string) => currentTargetId._set(blockId),
        };
    }),
    pickMime: () => "audio/webm",
}));

describe("getVoiceSession — cached facade observes a live engine switch", () => {
    beforeEach(() => {
        vi.resetModules();
        engineSignal = createRoot(() => createSignal<string | null>("groq"));
    });
    afterEach(() => vi.clearAllMocks());

    it("a session cached once at 'component setup' still reflects a later engine change", async () => {
        const { getVoiceSession } = await import("./useVoiceInput");
        const { createWhisperVoiceSession } = await import("./whisperVoiceEngine");

        // Simulates MicButton.tsx: fetched once, never re-invoked.
        const voice = getVoiceSession();

        voice.registerPane("pane-a", { appendFinal: () => {}, setInterim: () => {} });
        voice.toggleListening();
        expect(createWhisperVoiceSession).toHaveBeenCalledTimes(1); // lazily built on first real use
        expect(voice.isListening()).toBe(true);
        expect(voice.currentTargetId()).toBe("pane-a");

        // The user flips Settings -> Recording's engine picker from groq to
        // whisper-local. No component re-fetches getVoiceSession() — this is
        // exactly the cached-reference scenario the earlier fix missed.
        createRoot(() => engineSignal[1]("whisper-local"));

        // The cached `voice` facade must now be backed by a NEW underlying
        // session, not the stale groq one — checked on next actual use.
        expect(voice.isListening()).toBe(false); // fresh session, nothing listening yet
        expect(voice.currentTargetId()).toBe(null);
        expect(createWhisperVoiceSession).toHaveBeenCalledTimes(2);

        voice.registerPane("pane-b", { appendFinal: () => {}, setInterim: () => {} });
        voice.toggleListening();
        expect(voice.isListening()).toBe(true);
        expect(voice.currentTargetId()).toBe("pane-b");

        // Same call again with no further engine change must NOT rebuild again.
        void getVoiceSession();
        expect(createWhisperVoiceSession).toHaveBeenCalledTimes(2);
    });

    it("returns the exact same object identity across engine changes (safe to cache)", async () => {
        const { getVoiceSession } = await import("./useVoiceInput");
        const first = getVoiceSession();
        createRoot(() => engineSignal[1]("whisper-local"));
        // Re-reading a signal on the SAME facade object is how a real caller
        // would observe the change — re-fetching getVoiceSession() itself
        // must also still be the same object, since some callers do that too.
        const second = getVoiceSession();
        expect(second).toBe(first);
    });
});
