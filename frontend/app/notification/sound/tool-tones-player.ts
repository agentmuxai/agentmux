// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tool-tones audio player — dedicated chain with a lowpass filter
 * and an independent gain knob, sitting below the v1 master gain.
 *
 * Spec: docs/specs/SPEC_AGENT_TOOL_CALL_TONES_2026_06_05.md §5.
 *
 * Chain:
 *   per-tone OscillatorNode → per-tone envelope (GainNode)
 *     → tool-tones gain (settings-bound)
 *     → BiquadFilter (lowpass ~2.5 kHz)
 *     → v1 master gain
 *     → AudioContext.destination
 *
 * The lowpass softens onsets so rapid syllables blend into ambient
 * texture rather than poking through it; the independent gain lets
 * the user dial tool tones way down without quieting notifications.
 *
 * Idempotent attach — safe to call repeatedly (e.g. if the AudioContext
 * is rebuilt). State (`lastFiredAt`) survives across attaches by
 * design: coalesce semantics should not depend on context lifecycle.
 */

import { paramsForTool, type SyllableParams } from "./tool-tones";
import { DEFAULT_TOOLTONES_VOLUME } from "./sound-defaults";

/** Coalesce window per tool, in ms. See spec §9.4. */
export const TOOL_TONE_COALESCE_MS = 30;

/**
 * Per-tone envelope: ramp from the floor to the peak over the attack, then
 * back to the floor at the tone's end. Exported so flash-patterns.ts derives
 * each flash strike's intensity from the same numbers the synth plays.
 */
export const TOOL_TONE_ENVELOPE_PEAK = 0.4;
export const TOOL_TONE_ENVELOPE_FLOOR = 0.0001;
export const TOOL_TONE_ATTACK_S = 0.006;

export class ToolTonesPlayer {
    private filter: BiquadFilterNode | null = null;
    private gain: GainNode | null = null;
    private toolGainValue = DEFAULT_TOOLTONES_VOLUME;
    private lastFiredAt = new Map<string, number>();

    /**
     * Wire the chain into the given AudioContext and master GainNode.
     * Idempotent — calling twice rebuilds the chain against the same
     * (or a fresh) context.
     */
    attach(ctx: AudioContext, master: GainNode): void {
        const filter = ctx.createBiquadFilter();
        filter.type = "lowpass";
        filter.frequency.value = 2500;
        // Butterworth Q — no resonance peak, gentle rolloff.
        filter.Q.value = 0.707;
        const gain = ctx.createGain();
        gain.gain.value = this.toolGainValue;
        gain.connect(filter).connect(master);
        this.filter = filter;
        this.gain = gain;
    }

    /** True iff `attach()` has been called and the chain is wired up. */
    isAttached(): boolean {
        return this.gain != null;
    }

    /**
     * Set the tool-tones independent gain (0–1). Layered below the
     * shared master gain — turning the master to 0 silences both.
     */
    setVolume(value: number): void {
        const clamped = Math.max(0, Math.min(1, value));
        this.toolGainValue = clamped;
        if (this.gain) this.gain.gain.value = clamped;
    }

    /**
     * Play the syllable for `tool` through the chain. No-op if not
     * attached. Coalesces a second fire of the same tool within
     * `TOOL_TONE_COALESCE_MS`.
     *
     * Returns the AudioContext time the first tone was scheduled at, or
     * null when nothing played. The activity flash uses it to line its
     * pulses up with what is actually heard.
     */
    play(ctx: AudioContext, tool: string): number | null {
        const out = this.gain;
        if (!out) return null;
        const now =
            typeof performance !== "undefined" && performance.now
                ? performance.now()
                : Date.now();
        const last = this.lastFiredAt.get(tool) ?? 0;
        if (now - last < TOOL_TONE_COALESCE_MS) return null;
        this.lastFiredAt.set(tool, now);
        return playSyllable(ctx, out, paramsForTool(tool));
    }

    /** Test/dev helper — clear the coalesce map. */
    __resetCoalesce(): void {
        this.lastFiredAt.clear();
    }
}

function playSyllable(
    ctx: AudioContext,
    out: AudioNode,
    p: SyllableParams,
): number {
    const startAt = ctx.currentTime;
    const stepSec = (p.durationMs + p.gapMs) / 1000;
    const toneSec = p.durationMs / 1000;
    for (let i = 0; i < p.tones.length; i++) {
        const at = startAt + i * stepSec;
        const osc = ctx.createOscillator();
        const env = ctx.createGain();
        osc.type = p.wave;
        osc.frequency.setValueAtTime(p.tones[i], at);
        env.gain.setValueAtTime(TOOL_TONE_ENVELOPE_FLOOR, at);
        env.gain.exponentialRampToValueAtTime(TOOL_TONE_ENVELOPE_PEAK, at + TOOL_TONE_ATTACK_S);
        env.gain.exponentialRampToValueAtTime(TOOL_TONE_ENVELOPE_FLOOR, at + toneSec);
        osc.connect(env).connect(out);
        osc.start(at);
        osc.stop(at + toneSec + 0.02);
    }
    return startAt;
}
