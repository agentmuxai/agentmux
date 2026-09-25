// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Sound → flash patterns: strike timing and intensity derived from the synth
 * params, and the audible-time delay.
 * Spec: docs/specs/SPEC_AGENT_ACTIVITY_FLASH_SOUND_SYNC_2026_09_24.md §2, §3.1,
 * §3.2, §3.6.
 */

import { describe, expect, it } from "vitest";
import { FLASH_VISUAL_LEAD_MS } from "../../activity-flash";
import {
    audibleFlashDelayMs,
    categoryStrikeLevelDb,
    FLASH_MAX_AUDIO_DELAY_MS,
    FLASH_MIN_INTENSITY,
    flashPatternForSyllable,
    intensityForLevel,
    syllableStrikeLevelDb,
} from "../flash-patterns";
import { __getCurated, paramsForTool } from "../tool-tones";

const onsets = (tool: string) => flashPatternForSyllable(paramsForTool(tool)).strikes.map((s) => s.atMs);

describe("flashPatternForSyllable", () => {
    it("puts one strike on each tone's own onset (spec §2.1 table)", () => {
        expect(onsets("Read")).toEqual([0, 74]);
        expect(onsets("Grep")).toEqual([0, 63]);
        expect(onsets("Glob")).toEqual([0, 63]);
        expect(onsets("Edit")).toEqual([0, 62, 124]);
        expect(onsets("Write")).toEqual([0, 78]);
        expect(onsets("Bash")).toEqual([0, 88]);
        expect(onsets("Task")).toEqual([0, 69]);
        expect(onsets("Agent")).toEqual([0, 69, 138]);
    });

    it("matches every curated syllable's tone count and spacing", () => {
        for (const [tool, p] of Object.entries(__getCurated())) {
            expect(onsets(tool)).toEqual(p.tones.map((_, i) => i * (p.durationMs + p.gapMs)));
        }
    });

    it("gives a hashed (unknown) tool two strikes", () => {
        expect(onsets("mcp__weatherserver__get_forecast")).toHaveLength(2);
        expect(onsets("TodoWrite")).toEqual([0, 66]);
    });

    it("gives tool strikes about half strength: tool tones are 17–21.5 dB below event sounds", () => {
        // Curated tools sit at −18.1 to −19.5 dB; hashed syllables span
        // −17.0 (76 ms sine) to −21.3 (45 ms triangle).
        for (const tool of ["Read", "Edit", "Bash", "Grep", "TodoWrite", "mcp__weatherserver__get_forecast"]) {
            const level = syllableStrikeLevelDb(paramsForTool(tool));
            expect(level).toBeLessThan(-16.9);
            expect(level).toBeGreaterThan(-21.5);
            const strikes = flashPatternForSyllable(paramsForTool(tool)).strikes;
            for (const s of strikes) {
                expect(s.intensity).toBe(intensityForLevel(level));
                expect(s.intensity).toBeGreaterThanOrEqual(FLASH_MIN_INTENSITY);
                expect(s.intensity).toBeLessThan(0.56);
            }
        }
    });
});

describe("strike levels and intensity (spec §2.2, §3.2)", () => {
    it("puts the loudest event sounds at 0 dB and the single strikes just below", () => {
        expect(Math.max(categoryStrikeLevelDb("success"), categoryStrikeLevelDb("error"))).toBeCloseTo(0, 5);
        expect(categoryStrikeLevelDb("success")).toBeCloseTo(-0.1, 1);
        expect(categoryStrikeLevelDb("info")).toBeCloseTo(-1.9, 1);
        expect(categoryStrikeLevelDb("warning")).toBeCloseTo(-1.9, 1);
    });

    it("maps level to intensity: 2^(dB/20), clamped to [floor, 1]", () => {
        expect(intensityForLevel(0)).toBe(1);
        expect(intensityForLevel(5)).toBe(1);
        expect(intensityForLevel(-1.9)).toBeCloseTo(0.94, 2);
        expect(intensityForLevel(-12)).toBeCloseTo(0.66, 2);
        expect(intensityForLevel(-19)).toBeCloseTo(0.52, 2);
        expect(intensityForLevel(-25)).toBe(FLASH_MIN_INTENSITY);
    });
});

describe("audibleFlashDelayMs", () => {
    const ctxWith = (over: Partial<AudioContext>): AudioContext =>
        ({ currentTime: 10, baseLatency: 0, outputLatency: 0, ...over }) as unknown as AudioContext;

    it("uses the output timestamp: the page time the device plays a context time at", () => {
        // Context time 10.000 s is heard at page time 5000 ms; the sound is
        // scheduled at 10.050 s, so it is heard at 5050 ms.
        const ctx = ctxWith({ getOutputTimestamp: () => ({ contextTime: 10, performanceTime: 5000 }) });
        expect(audibleFlashDelayMs(ctx, 10.05, 4990)).toBeCloseTo(60 - FLASH_VISUAL_LEAD_MS, 5);
    });

    it("falls back to the context's latency estimates before it has rendered", () => {
        const ctx = ctxWith({
            baseLatency: 0.01,
            outputLatency: 0.03,
            getOutputTimestamp: () => ({ contextTime: 0, performanceTime: 0 }),
        });
        // Scheduled 5 ms after currentTime, plus 40 ms of latency.
        expect(audibleFlashDelayMs(ctx, 10.005, 1000)).toBeCloseTo(45 - FLASH_VISUAL_LEAD_MS, 5);
    });

    it("never goes negative, never waits past the cap, and ignores garbage", () => {
        expect(audibleFlashDelayMs(ctxWith({}), 10, 1000)).toBe(0);
        expect(audibleFlashDelayMs(ctxWith({ outputLatency: 3 }), 10, 1000)).toBe(FLASH_MAX_AUDIO_DELAY_MS);
        expect(audibleFlashDelayMs(ctxWith({ outputLatency: Number.NaN }), 10, 1000)).toBe(0);
    });
});
