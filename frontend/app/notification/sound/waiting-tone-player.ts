// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Waiting-tone player — looping C→E→G arpeggio that plays while an agent
 * pane is blocked waiting for user input.
 *
 * Spec: docs/specs/SPEC_AGENT_WAITING_AMBIENT_SOUND_2026_06_19.md §5–§6.
 *
 * Chain:
 *   per-note OscillatorNode → per-note envelope (GainNode)
 *     → BiquadFilter (lowpass ~1200 Hz — keeps it mellow)
 *     → waiting gain (settings-bound, default 0.25)
 *     → v1 master gain
 *     → AudioContext.destination
 *
 * Lifecycle: `start()` is idempotent; `stop()` fades out over 600 ms and
 * resolves when the fade is complete. The player does NOT self-dispose the
 * AudioContext — the context is owned by SoundPlayer.
 */

import { DEFAULT_WAITING_VOLUME } from "./sound-defaults";

/** C5 → E5 → G5 in Hz (major triad arpeggio). */
const ARPEGGIO_HZ = [523.25, 659.25, 783.99] as const;
/** Duration of each note (ms). */
const NOTE_DURATION_MS = 300;
/** Gap between notes (ms). */
const NOTE_GAP_MS = 200;
/** Pause after the full triad before looping (ms). */
const LOOP_PAUSE_MS = 1000;
/** Fade-in ramp (s). */
const FADE_IN_S = 0.4;
/** Fade-out ramp (s). */
const FADE_OUT_S = 0.6;
/** Peak oscillator gain inside one note, before chain gain. */
const NOTE_PEAK = 0.35;

export class WaitingTonePlayer {
    private ctx: AudioContext | null = null;
    private masterGain: GainNode | null = null;
    private filter: BiquadFilterNode | null = null;
    private gain: GainNode | null = null;
    private gainValue = DEFAULT_WAITING_VOLUME;
    private running = false;
    private scheduleHandle: ReturnType<typeof setTimeout> | null = null;

    /**
     * Wire the chain into the given AudioContext and master GainNode.
     * Must be called before `start()`. Safe to call again if the context
     * is rebuilt — tears down and rebuilds the chain.
     */
    attach(ctx: AudioContext, master: GainNode): void {
        this.ctx = ctx;
        this.masterGain = master;
        const filter = ctx.createBiquadFilter();
        filter.type = "lowpass";
        filter.frequency.value = 1200;
        filter.Q.value = 0.7;
        const gain = ctx.createGain();
        gain.gain.value = 0; // start silent; fade in on start()
        gain.connect(filter).connect(master);
        this.filter = filter;
        this.gain = gain;
    }

    isAttached(): boolean {
        return this.gain != null;
    }

    setVolume(value: number): void {
        const clamped = Math.max(0, Math.min(1, value));
        this.gainValue = clamped;
        // Only apply immediately if we're mid-loop (running). Otherwise
        // start() will apply it via the fade-in ramp.
        if (this.running && this.gain) {
            this.gain.gain.value = clamped;
        }
    }

    /**
     * Start the looping arpeggio. Idempotent — no-op if already running.
     * Fades in over FADE_IN_S seconds.
     */
    start(volume?: number): void {
        if (!this.ctx || !this.gain) return;
        if (this.running) return;
        if (volume !== undefined) this.gainValue = Math.max(0, Math.min(1, volume));
        this.running = true;

        const ctx = this.ctx;
        if (ctx.state === "suspended") {
            void ctx.resume().catch(() => { /* graceful degrade */ });
        }

        // Fade in
        const gain = this.gain;
        gain.gain.cancelScheduledValues(ctx.currentTime);
        gain.gain.setValueAtTime(0.0001, ctx.currentTime);
        gain.gain.linearRampToValueAtTime(this.gainValue, ctx.currentTime + FADE_IN_S);

        this.scheduleNextCycle();
    }

    /**
     * Fade out and stop. Returns a Promise that resolves after the fade
     * completes. Safe to call when already stopped.
     */
    stop(): Promise<void> {
        if (!this.running) return Promise.resolve();
        this.running = false;
        if (this.scheduleHandle !== null) {
            clearTimeout(this.scheduleHandle);
            this.scheduleHandle = null;
        }
        if (!this.ctx || !this.gain) return Promise.resolve();
        const gain = this.gain;
        const ctx = this.ctx;
        gain.gain.cancelScheduledValues(ctx.currentTime);
        gain.gain.setValueAtTime(gain.gain.value, ctx.currentTime);
        gain.gain.linearRampToValueAtTime(0.0001, ctx.currentTime + FADE_OUT_S);
        return new Promise((resolve) => {
            setTimeout(resolve, FADE_OUT_S * 1000 + 50);
        });
    }

    /** Test/dev helper. */
    __isRunning(): boolean {
        return this.running;
    }

    private scheduleNextCycle(): void {
        if (!this.running) return;
        const ctx = this.ctx!;
        const gain = this.gain!;
        const stepSec = (NOTE_DURATION_MS + NOTE_GAP_MS) / 1000;
        const noteSec = NOTE_DURATION_MS / 1000;
        const startAt = ctx.currentTime;

        for (let i = 0; i < ARPEGGIO_HZ.length; i++) {
            const at = startAt + i * stepSec;
            const osc = ctx.createOscillator();
            const env = ctx.createGain();
            osc.type = "sine";
            osc.frequency.setValueAtTime(ARPEGGIO_HZ[i], at);
            env.gain.setValueAtTime(0.0001, at);
            env.gain.exponentialRampToValueAtTime(NOTE_PEAK, at + 0.02);
            env.gain.exponentialRampToValueAtTime(0.0001, at + noteSec);
            osc.connect(env).connect(gain);
            osc.start(at);
            osc.stop(at + noteSec + 0.02);
        }

        // Total cycle: 3 notes + gaps + pause before loop
        const cycleDurationMs =
            ARPEGGIO_HZ.length * (NOTE_DURATION_MS + NOTE_GAP_MS) + LOOP_PAUSE_MS;

        this.scheduleHandle = setTimeout(() => {
            this.scheduleHandle = null;
            this.scheduleNextCycle();
        }, cycleDurationMs);
    }
}
