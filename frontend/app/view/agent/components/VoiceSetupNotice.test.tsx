// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

/**
 * The composer's "Voice input isn't set up" line: shown in the pane whose mic was asked for and in no
 * other, in the person's words for each thing that can be missing, with the way to Settings > Recording
 * and a Dismiss that also gives the mic its plain look back.
 */
import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";
import type { VoiceSetupGap } from "@/app/hook/useVoiceInput";

const h = vi.hoisted(() => ({ openSettingsAt: vi.fn(async () => {}), voice: {} as Record<string, unknown> }));
vi.mock("@/app/view/settings/open-settings", () => ({ openSettingsAt: h.openSettingsAt }));
vi.mock("@/app/hook/useVoiceInput", () => ({ getVoiceSession: () => h.voice }));

import { VoiceSetupNotice, voiceSetupText } from "./VoiceSetupNotice";

const atom = <T,>(initial: T) => {
    const [get, set] = createSignal<T>(initial);
    return Object.assign(get, { _set: (v: T) => void set(() => v) });
};
let notSetUp: ReturnType<typeof atom<VoiceSetupGap | null>>;
let lastError: ReturnType<typeof atom<string | null>>;
const gap = (over: Partial<VoiceSetupGap> = {}): VoiceSetupGap => ({ blockId: "blk", missing: "groq-key", detail: "srv's own words", ...over });

beforeEach(() => {
    h.openSettingsAt.mockClear();
    notSetUp = atom<VoiceSetupGap | null>(null);
    lastError = atom<string | null>(null);
    h.voice = { notSetUp, lastError };
});
afterEach(() => cleanup());

describe("VoiceSetupNotice", () => {
    it("shows nothing until this pane's mic was refused for a missing engine", () => {
        render(() => <VoiceSetupNotice blockId="blk" />);
        expect(screen.queryByRole("status")).toBeNull();
        notSetUp._set(gap());
        expect(screen.getByRole("status").textContent).toContain("Voice input isn't set up");
        expect(screen.getByRole("status").textContent).toContain("add a Groq key, or choose local Whisper");
    });

    it("stays out of a pane whose mic was not the one asked for", () => {
        notSetUp._set(gap({ blockId: "other" }));
        render(() => <VoiceSetupNotice blockId="blk" />);
        expect(screen.queryByRole("status")).toBeNull();
    });

    it("shows nothing with an engine that reports no gap at all", () => {
        h.voice = { lastError };
        render(() => <VoiceSetupNotice blockId="blk" />);
        expect(screen.queryByRole("status")).toBeNull();
    });

    it("takes the person to Settings > Recording", () => {
        notSetUp._set(gap());
        render(() => <VoiceSetupNotice blockId="blk" />);
        fireEvent.click(screen.getByRole("button", { name: "Open Recording settings" }));
        expect(h.openSettingsAt).toHaveBeenCalledWith("recording");
        // Still there: the person may come back to it.
        expect(screen.getByRole("status")).toBeTruthy();
    });

    it("goes on Dismiss, and the mic drops its blocked look", () => {
        notSetUp._set(gap());
        lastError._set("not-configured");
        render(() => <VoiceSetupNotice blockId="blk" />);
        fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
        expect(screen.queryByRole("status")).toBeNull();
        expect(notSetUp()).toBeNull();
        expect(lastError()).toBeNull();
    });

    it("leaves another error on the mic alone when dismissed", () => {
        notSetUp._set(gap());
        lastError._set("not-allowed");
        render(() => <VoiceSetupNotice blockId="blk" />);
        fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
        expect(lastError()).toBe("not-allowed");
    });
});

describe("voiceSetupText", () => {
    it("says what each missing thing is in plain words, never srv's settings keys", () => {
        expect(voiceSetupText({ missing: "groq-key" })).toBe("It needs a transcription engine: add a Groq key, or choose local Whisper.");
        expect(voiceSetupText({ missing: "whisper-cli" })).toContain("path to whisper-cli");
        expect(voiceSetupText({ missing: "whisper-model" })).toContain("model file wasn't found");
        // A kind this build does not know reads as the general case.
        expect(voiceSetupText({ missing: "engine" })).toBe(voiceSetupText({ missing: "groq-key" }));
        for (const missing of ["groq-key", "whisper-cli", "whisper-model"]) expect(voiceSetupText({ missing })).not.toContain("voice:");
    });
});
