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
