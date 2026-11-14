// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The level a mic button shows for a microphone reading (`micLevelOf`). */
import { describe, expect, it } from "vitest";
import { micLevelOf } from "./audioLevel";

describe("micLevelOf", () => {
    it("shows nothing for silence and for the room's own noise", () => {
        for (const rms of [0, 0.001, 0.004, 0.006]) expect(micLevelOf(rms)).toBe(0);
    });

    it("shows a little where the engine starts to count a sound as speech", () => {
        // 0.012 is the engine's own speech threshold (whisperVoiceEngine.ts SILENCE_RMS).
        expect(micLevelOf(0.012)).toBe(0.2);
    });

    it("rises with the sound and is full for a loud voice, never more", () => {
        const readings = [0.012, 0.03, 0.06, 0.1, 0.156];
        const levels = readings.map(micLevelOf);
        for (let i = 1; i < levels.length; i++) expect(levels[i]).toBeGreaterThan(levels[i - 1]);
        expect(levels[levels.length - 1]).toBe(1);
        expect(micLevelOf(0.9)).toBe(1);
    });

    it("moves in steps of a twentieth, so a reading that barely changed is the same level", () => {
        expect(micLevelOf(0.05)).toBe(micLevelOf(0.0505));
        for (const rms of [0.02, 0.05, 0.08]) expect(Number.isInteger(Math.round(micLevelOf(rms) * 20 * 1e6) / 1e6)).toBe(true);
    });
});
