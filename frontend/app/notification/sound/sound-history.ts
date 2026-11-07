// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What this window played lately, so the Audit can answer "who was that" (docs/specs/SPEC_AUDIT_LIVE_LOG_OF_EVERY_AGENT_ACTION_2026_11_03.md §3.3).
// The two places that play a sound (the event player and the tool tones) note it here, past every gate the
// sound itself passed. In memory, per window, and short: it is only read for the last minute. No SolidJS
// coupling, as with the sound-event bus: a reader that wants a signal makes one from `subscribePlayedSounds`.

import type { SoundId } from "./sounds";

export interface PlayedSound {
    /** The pane whose agent caused it. */
    blockId: string;
    /** An event sound, or `tool` for the tone of a tool call. */
    event: SoundId | "tool";
    /** The tool, for a tool tone. */
    tool?: string;
    /** Unix ms. */
    at: number;
}

/** How many are kept. */
export const SOUND_RING = 50;

let played: readonly PlayedSound[] = [];
const listeners = new Set<() => void>();

/** The sounds played, oldest first. */
export function recentSounds(): readonly PlayedSound[] {
    return played;
}

export function noteSound(sound: PlayedSound): void {
    played = [...played, sound].slice(-SOUND_RING);
    for (const listener of listeners) listener();
}

/** Calls `listener` whenever a sound is noted. Returns the way to stop. */
export function subscribePlayedSounds(listener: () => void): () => void {
    listeners.add(listener);
    return () => void listeners.delete(listener);
}

/** Test helper: forget what played. */
export function __resetPlayedSounds(): void {
    played = [];
    for (const listener of listeners) listener();
}
