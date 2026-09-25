// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Sound → flash: the activity flash's strike pattern for a sound, derived
 * from the same parameters the synth plays, so no timing or level number is
 * written twice.
 *
 * Spec: docs/specs/SPEC_AGENT_ACTIVITY_FLASH_SOUND_SYNC_2026_09_24.md §3.1,
 * §3.2, §3.6.
 */

import { FLASH_VISUAL_LEAD_MS, type FlashPattern } from "@/app/notification/activity-flash";
import { DEFAULT_MASTER_VOLUME, DEFAULT_TOOLTONES_VOLUME } from "./sound-defaults";
import type { SoundCategory } from "./sounds";
import { SYNTH_ATTACK_S, SYNTH_DECAY_END_S, SYNTH_ENVELOPE_FLOOR, synthParamsFor } from "./synth-fallback";
import type { SyllableParams } from "./tool-tones";
import { TOOL_TONE_ATTACK_S, TOOL_TONE_ENVELOPE_FLOOR, TOOL_TONE_ENVELOPE_PEAK } from "./tool-tones-player";

/** Quietest strike's flash intensity, so the quietest sound still shows. */
export const FLASH_MIN_INTENSITY = 0.5;

/**
 * Largest offset, either way, the flash will take from audio timing. Ahead:
 * Bluetooth output can legitimately run 150–300 ms; beyond this, a reported
 * latency is more likely wrong than real, and a flash that late no longer
 * reads as "that one, just now". Behind: a sound can only have started a
 * few tens of ms before its flash is asked for (the 30 ms coalesce window
 * plus the visual lead), so anything larger is bad clock data.
 */
export const FLASH_MAX_AUDIO_DELAY_MS = 500;

// ── Strike level ─────────────────────────────────────────────────────────

/** RMS of each waveform at unit amplitude. */
const WAVE_RMS: Record<OscillatorType, number> = {
    sine: Math.SQRT1_2,
    triangle: 1 / Math.sqrt(3),
    sawtooth: 1 / Math.sqrt(3),
    square: 1,
    custom: Math.SQRT1_2,
};

interface StrikeShape {
    wave: OscillatorType;
    /** Envelope peak × every gain stage after it, at default volumes. */
    amplitude: number;
    /** Envelope peak and floor, before any gain stage: they set the decay rate. */
    envelopePeak: number;
    envelopeFloor: number;
    attackMs: number;
    /** Onset to the moment the decay reaches the floor. */
    decayEndMs: number;
}

/**
 * Energy of one exponentially decaying strike, in dB (arbitrary reference).
 * With time constant τ = decay length / ln(peak / floor), the energy is
 * (amplitude × RMS)² × τ / 2: louder and longer-ringing strikes both count.
 */
function strikeEnergyDb(s: StrikeShape): number {
    const tau = (s.decayEndMs - s.attackMs) / Math.log(s.envelopePeak / s.envelopeFloor);
    const rmsAmplitude = s.amplitude * WAVE_RMS[s.wave];
    return 10 * Math.log10((rmsAmplitude * rmsAmplitude * tau) / 2);
}

function syllableStrike(p: SyllableParams): StrikeShape {
    return {
        wave: p.wave,
        amplitude: TOOL_TONE_ENVELOPE_PEAK * DEFAULT_TOOLTONES_VOLUME * DEFAULT_MASTER_VOLUME,
        envelopePeak: TOOL_TONE_ENVELOPE_PEAK,
        envelopeFloor: TOOL_TONE_ENVELOPE_FLOOR,
        attackMs: TOOL_TONE_ATTACK_S * 1000,
        decayEndMs: p.durationMs,
    };
}

function categoryStrike(c: SoundCategory): StrikeShape {
    const p = synthParamsFor(c);
    return {
        wave: p.wave,
        amplitude: p.peak * DEFAULT_MASTER_VOLUME,
        envelopePeak: p.peak,
        envelopeFloor: SYNTH_ENVELOPE_FLOOR,
        attackMs: SYNTH_ATTACK_S * 1000,
        decayEndMs: SYNTH_DECAY_END_S * 1000,
    };
}

// `satisfies` makes adding a SoundCategory without listing it here a type error.
const CATEGORIES = Object.keys({ success: 1, info: 1, warning: 1, error: 1 } satisfies Record<
    SoundCategory,
    1
>) as SoundCategory[];

/** The loudest strike in the app: always an event sound (no 0.25 tool-tones stage). */
let referenceDb: number | null = null;
function loudestStrikeDb(): number {
    referenceDb ??= Math.max(...CATEGORIES.map((c) => strikeEnergyDb(categoryStrike(c))));
    return referenceDb;
}

/**
 * Flash intensity for a strike `deltaDb` below the loudest sound. 2^(Δ/10)
 * is perceived loudness (+10 dB ≈ twice as loud); halving the exponent
 * reflects that brightness perception compresses more than loudness does,
 * and keeps quiet sounds visible. Spec §3.2.
 */
export function intensityForLevel(deltaDb: number): number {
    return Math.min(1, Math.max(FLASH_MIN_INTENSITY, Math.pow(2, deltaDb / 20)));
}

/** A tool syllable's strike level relative to the loudest sound, in dB (≤ 0 in practice). */
export function syllableStrikeLevelDb(p: SyllableParams): number {
    return strikeEnergyDb(syllableStrike(p)) - loudestStrikeDb();
}

/** An event sound's strike level relative to the loudest sound, in dB. */
export function categoryStrikeLevelDb(c: SoundCategory): number {
    return strikeEnergyDb(categoryStrike(c)) - loudestStrikeDb();
}

// ── Patterns ─────────────────────────────────────────────────────────────

/**
 * One strike per tone, at the tone's own onset (`tool-tones-player.ts`
 * schedules tone i at i × (duration + gap)). Every tone in a syllable has
 * the same envelope, so they share one intensity.
 */
export function flashPatternForSyllable(p: SyllableParams): FlashPattern {
    const intensity = intensityForLevel(syllableStrikeLevelDb(p));
    const step = p.durationMs + p.gapMs;
    return { strikes: p.tones.map((_, i) => ({ atMs: i * step, intensity })) };
}

// ── Timing ───────────────────────────────────────────────────────────────

/**
 * When, relative to `nowMs`, a flash's first strike should be on screen for
 * a sound scheduled at AudioContext time `startAt`: positive means wait that
 * long; negative means the sound is already that far in (a call coalesced
 * into a syllable that is already playing, or an output faster than the
 * visual lead), so the pattern resumes mid-way rather than restarting behind
 * the sound (Codex P2 on #3717).
 *
 * `getOutputTimestamp()` pairs a context time with the page time the output
 * device plays it at, so it already includes the device's output latency.
 * Before the context has rendered anything it reports zeros; then fall back
 * to the context's own latency estimates.
 */
export function audibleFlashDelayMs(ctx: AudioContext, startAt: number, nowMs: number): number {
    let audibleAt: number;
    const ts = typeof ctx.getOutputTimestamp === "function" ? ctx.getOutputTimestamp() : null;
    if (ts?.performanceTime && ts.contextTime != null) {
        audibleAt = ts.performanceTime + (startAt - ts.contextTime) * 1000;
    } else {
        const latencyS = (ctx.baseLatency ?? 0) + (ctx.outputLatency ?? 0);
        audibleAt = nowMs + (startAt - ctx.currentTime + latencyS) * 1000;
    }
    const delay = audibleAt - nowMs - FLASH_VISUAL_LEAD_MS;
    if (!Number.isFinite(delay)) return 0;
    return Math.min(FLASH_MAX_AUDIO_DELAY_MS, Math.max(-FLASH_MAX_AUDIO_DELAY_MS, delay));
}
