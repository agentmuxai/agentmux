// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Synth fallback — oscillator + exponential-decay envelope per
 * SoundCategory. Used when no asset is shipped (v1 default) or when
 * an asset fails to load. Polite by design: ~150ms total duration,
 * gentle attack, sub-1.0 peak gain.
 *
 * Spec: SPEC_SOUND_NOTIFICATIONS_2026_06_05.md §4.5 + Appendix A.
 *
 * The whole module is allocation-light — every play creates one
 * oscillator + one gain node, both single-use as recommended by
 * MDN's Web Audio best practices. No shared mutable state.
 */

import type { SoundCategory } from "./sounds";

export interface SynthParams {
    wave: OscillatorType;
    freq: number;
    /** Peak envelope gain, 0–1. Multiplied with the caller's `gain`. */
    peak: number;
    /** Optional second tone, played 30ms after the first for a "ding" feel. */
    second?: { freq: number; delayMs: number };
}

/** Envelope shape shared by every synth tone (exported for flash-patterns.ts). */
export const SYNTH_ENVELOPE_FLOOR = 0.0001;
export const SYNTH_ATTACK_S = 0.01;
/** When the decay reaches the floor, measured from the tone's onset. */
export const SYNTH_DECAY_END_S = 0.15;
const SYNTH_STOP_S = 0.18;

export function synthParamsFor(category: SoundCategory): SynthParams {
    switch (category) {
        case "success":
            // Two-tone rising sine — "good news" interval (G5 → C6).
            return {
                wave: "sine",
                freq: 784,
                peak: 0.5,
                second: { freq: 1046, delayMs: 70 },
            };
        case "info":
            return { wave: "sine", freq: 660, peak: 0.4 };
        case "warning":
            return { wave: "triangle", freq: 440, peak: 0.5 };
        case "error":
            // Two-tone falling square — "minor concern" interval (A4 → D4).
            return {
                wave: "square",
                freq: 440,
                peak: 0.35,
                second: { freq: 294, delayMs: 90 },
            };
    }
}

function playTone(
    ctx: AudioContext,
    out: AudioNode,
    wave: OscillatorType,
    freq: number,
    peak: number,
    startAt: number,
): void {
    const osc = ctx.createOscillator();
    const env = ctx.createGain();
    osc.type = wave;
    osc.frequency.setValueAtTime(freq, startAt);
    env.gain.setValueAtTime(SYNTH_ENVELOPE_FLOOR, startAt);
    env.gain.exponentialRampToValueAtTime(Math.max(SYNTH_ENVELOPE_FLOOR, peak), startAt + SYNTH_ATTACK_S);
    env.gain.exponentialRampToValueAtTime(SYNTH_ENVELOPE_FLOOR, startAt + SYNTH_DECAY_END_S);
    osc.connect(env).connect(out);
    osc.start(startAt);
    osc.stop(startAt + SYNTH_STOP_S);
}

/**
 * Play a polite tone matching the category through `out`. `gain`
 * scales the per-tone peak. Returns immediately — the audio plays
 * asynchronously and self-disposes. The return value is the context
 * time the first tone was scheduled at (the activity flash times itself
 * from it).
 */
export function playSynthFallback(
    ctx: AudioContext,
    out: AudioNode,
    category: SoundCategory,
    gain: number,
): number {
    const now = ctx.currentTime;
    const p = synthParamsFor(category);
    const scaledPeak = p.peak * Math.max(0, Math.min(1, gain));
    playTone(ctx, out, p.wave, p.freq, scaledPeak, now);
    if (p.second) {
        playTone(ctx, out, p.wave, p.second.freq, scaledPeak, now + p.second.delayMs / 1000);
    }
    return now;
}
