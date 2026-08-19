// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Sound player — owns the AudioContext, the per-id AudioBuffer cache,
 * and the master GainNode. One instance per renderer (the sound
 * service holds it). Plays via `AudioBufferSourceNode` when an asset
 * is loaded; falls back to the synth tone otherwise.
 *
 * Spec: SPEC_SOUND_NOTIFICATIONS_2026_06_05.md §4.5.
 *
 * Autoplay policy: Chromium blocks new AudioContexts from playing
 * before a user gesture. `prime()` MUST be called from inside a
 * user-gesture event handler (typically the first pointerdown /
 * keydown after app load — wired in `sound-service.ts`).
 */

import { playSynthFallback } from "./synth-fallback";
import type { SoundDef } from "./sounds";
import { SOUNDS } from "./sounds";
import { DEFAULT_MASTER_VOLUME } from "./sound-defaults";

export class SoundPlayer {
    private ctx: AudioContext | null = null;
    private masterGain: GainNode | null = null;
    private buffers = new Map<string, AudioBuffer>();
    private primed = false;
    private masterGainValue = DEFAULT_MASTER_VOLUME;

    /** Whether the AudioContext has been created. */
    isPrimed(): boolean {
        return this.primed;
    }

    /**
     * Direct access to the AudioContext. Returns null before `prime()`
     * has been called. Used by sibling subsystems (e.g. the tool-tones
     * player) to hook into the same context.
     */
    getAudioContext(): AudioContext | null {
        return this.ctx;
    }

    /**
     * Direct access to the master GainNode. Returns null before
     * `prime()` has been called. Used by sibling subsystems to layer
     * additional gain/filter chains below the master volume.
     */
    getMasterGain(): GainNode | null {
        return this.masterGain;
    }

    /**
     * Initialize the AudioContext and preload assets. Idempotent.
     * Must be invoked from a user-gesture handler under Chromium's
     * autoplay policy.
     */
    async prime(): Promise<void> {
        if (this.primed) return;
        const Ctor =
            (window as unknown as { AudioContext?: typeof AudioContext })
                .AudioContext ??
            (window as unknown as { webkitAudioContext?: typeof AudioContext })
                .webkitAudioContext;
        if (!Ctor) {
            console.warn("[sound] AudioContext unavailable — sound disabled");
            return;
        }
        this.ctx = new Ctor();
        this.masterGain = this.ctx.createGain();
        this.masterGain.gain.value = this.masterGainValue;
        this.masterGain.connect(this.ctx.destination);
        this.primed = true;
        await Promise.all(
            Object.values(SOUNDS)
                .filter((s): s is SoundDef & { asset: string } => !!s.asset)
                .map((s) => this.loadAsset(s.asset)),
        );
    }

    setMasterGain(value: number): void {
        const clamped = Math.max(0, Math.min(1, value));
        this.masterGainValue = clamped;
        if (this.masterGain) this.masterGain.gain.value = clamped;
    }

    /**
     * Play the given sound definition through the master bus.
     * No-op if the player is not yet primed. Picks the buffer path
     * when the asset is loaded; otherwise routes to the synth fallback.
     */
    play(def: SoundDef, gain = 1): void {
        const ctx = this.ctx;
        const out = this.masterGain;
        if (!ctx || !out) return;
        if (ctx.state === "suspended") {
            // A best-effort resume — the autoplay-prime path should
            // have settled this already; if the context was suspended
            // by an OS interruption (e.g. screen lock), resume now.
            void ctx.resume().catch(() => {
                /* ignore — we'll just synth-fallback */
            });
        }
        const buf = def.asset ? this.buffers.get(def.asset) : undefined;
        if (buf) {
            const src = ctx.createBufferSource();
            src.buffer = buf;
            const perPlay = ctx.createGain();
            perPlay.gain.value = Math.max(0, Math.min(1, gain));
            src.connect(perPlay).connect(out);
            src.start();
            return;
        }
        playSynthFallback(ctx, out, def.category, gain);
    }

    private async loadAsset(asset: string): Promise<void> {
        if (!this.ctx) return;
        try {
            const resp = await fetch(`/${asset}`);
            if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
            const bytes = await resp.arrayBuffer();
            const buf = await this.ctx.decodeAudioData(bytes);
            this.buffers.set(asset, buf);
        } catch (e) {
            console.warn(`[sound] failed to load ${asset}; using synth fallback`, e);
        }
    }
}
