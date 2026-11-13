// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shared RMS-from-AnalyserNode computation. Factored out of
 * `whisperVoiceEngine.ts`'s `pollLevel()` (originally VAD-only, its number
 * was never rendered) so `useMicLevelMeter` can reuse the identical
 * calculation instead of duplicating the Web Audio boilerplate.
 * See docs/specs/SPEC_SETTINGS_RECORDING_INPUT_SECTION_2026_08_19.md §4.
 */
export function computeRms(analyser: AnalyserNode): number {
    const buf = new Uint8Array(analyser.fftSize);
    analyser.getByteTimeDomainData(buf);
    let sum = 0;
    for (let i = 0; i < buf.length; i++) {
        const v = (buf[i] - 128) / 128;
        sum += v * v;
    }
    return Math.sqrt(sum / buf.length);
}

/** Below this a reading is the room, not a voice: nothing is shown for it. */
const LEVEL_FLOOR_RMS = 0.006;
/** A reading this far above the floor fills the meter: a raised voice close to the microphone. */
const LEVEL_FULL_RMS = 0.15;
/** The steps the shown level moves in, so a reading that barely changed redraws nothing. */
const LEVEL_STEPS = 20;

/**
 * An RMS reading as the level a mic button shows, 0 to 1 in steps of 1/20: 0 for silence and the room's
 * own noise, about 0.2 where the engine starts to count a sound as speech, 1 for a loud voice. The square
 * root spreads quiet speech over the scale, where a straight line would leave it near the bottom.
 */
export function micLevelOf(rms: number): number {
    const above = Math.max(0, rms - LEVEL_FLOOR_RMS) / LEVEL_FULL_RMS;
    return Math.round(Math.min(1, Math.sqrt(above)) * LEVEL_STEPS) / LEVEL_STEPS;
}
