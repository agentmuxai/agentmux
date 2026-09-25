// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Shutdown-pending tone — something other than the user asked to shut an
 * agent down, and the user has 15 s to keep it
 * (docs/specs/SPEC_AGENT_SELF_QUIT_2026_09_24.md §6.5).
 *
 * The same chain as the waiting tone (waiting-tone-player.ts), so it reads as
 * "AgentMux needs you", but a falling minor arpeggio (A4 → F4 → D4) where the
 * question tone rises through a major triad. One-shot: the banner plays it
 * at the start and again with 5 s left.
 */

/** A4 → F4 → D4 in Hz (falling D minor triad). */
export const SHUTDOWN_ARPEGGIO_HZ = [440.0, 349.23, 293.66] as const;
const NOTE_DURATION_MS = 300;
const NOTE_GAP_MS = 200;
const NOTE_PEAK = 0.35;

export function playShutdownTone(ctx: AudioContext, master: GainNode, volume: number): void {
    if (ctx.state === "suspended") {
        void ctx.resume().catch(() => { /* graceful degrade */ });
    }
    const filter = ctx.createBiquadFilter();
    filter.type = "lowpass";
    filter.frequency.value = 1200;
    filter.Q.value = 0.7;
    const gain = ctx.createGain();
    gain.gain.value = Math.max(0, Math.min(1, volume));
    gain.connect(filter).connect(master);

    const stepSec = (NOTE_DURATION_MS + NOTE_GAP_MS) / 1000;
    const noteSec = NOTE_DURATION_MS / 1000;
    const startAt = ctx.currentTime;
    SHUTDOWN_ARPEGGIO_HZ.forEach((hz, i) => {
        const at = startAt + i * stepSec;
        const osc = ctx.createOscillator();
        const env = ctx.createGain();
        osc.type = "sine";
        osc.frequency.setValueAtTime(hz, at);
        env.gain.setValueAtTime(0.0001, at);
        env.gain.exponentialRampToValueAtTime(NOTE_PEAK, at + 0.02);
        env.gain.exponentialRampToValueAtTime(0.0001, at + noteSec);
        osc.connect(env).connect(gain);
        osc.start(at);
        osc.stop(at + noteSec + 0.02);
    });
    // Let the chain go once the last note has finished.
    const totalMs = SHUTDOWN_ARPEGGIO_HZ.length * (NOTE_DURATION_MS + NOTE_GAP_MS) + 100;
    setTimeout(() => gain.disconnect(), totalMs);
}
