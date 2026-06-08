// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Sound-event bus — typed, multicast, allocation-light.
 *
 * Spec: docs/specs/SPEC_SOUND_NOTIFICATIONS_2026_06_05.md §4.3.
 *
 * The bus is the **only** public surface most callers ever touch:
 *
 *     import { notify } from "@/app/notification/sound";
 *     notify("agent.turn.complete", { sourceBlockId });
 *
 * Subscribers (in practice: the sound service) call
 * `subscribeSoundEvents` and receive every event. A subscriber that
 * throws is isolated — it does not poison the other subscribers or
 * the caller's stack.
 *
 * No SolidJS coupling — the bus is a plain Set so it can be primed
 * during module load without a reactive root. The orchestrator
 * (`sound-service.ts`) is where settings, focus, and player
 * coordination live.
 */

import type { SoundId } from "./sounds";

export interface SoundEvent {
    id: SoundId;
    /**
     * blockId of the originating pane, if any. Used by the orchestrator
     * to suppress sound when the source pane is focused and the window
     * is in foreground.
     */
    sourceBlockId?: string;
    /**
     * Per-emit overrides. `gain` multiplies the master gain (0–1).
     * `asset` lets a caller request a non-default asset (currently
     * unused — synth-only in v1 — but the API surface is stable).
     */
    override?: { asset?: string; gain?: number };
}

type Listener = (ev: SoundEvent) => void;

const listeners = new Set<Listener>();

export function subscribeSoundEvents(listener: Listener): () => void {
    listeners.add(listener);
    return () => {
        listeners.delete(listener);
    };
}

export function notify(id: SoundId, opts?: Omit<SoundEvent, "id">): void {
    const ev: SoundEvent = { id, ...opts };
    for (const l of listeners) {
        try {
            l(ev);
        } catch (e) {
            console.warn(`[sound] listener threw for ${id}`, e);
        }
    }
}

/** Test/dev helper — wipe all subscribers. Never call in production. */
export function __resetSoundListeners(): void {
    listeners.clear();
}
