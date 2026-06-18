// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Sound registry — the single source of truth for every sound the app
 * can play. Adding a new sound is one entry here plus one settings key
 * (see SPEC_SOUND_NOTIFICATIONS_2026_06_05.md §5).
 *
 * Each entry is purely declarative. The orchestrator
 * (`sound-service.ts`) reads the registry and the user's settings to
 * decide whether and how to play. The player (`sound-player.ts`) reads
 * the asset path (if any) and falls back to a synthesized tone keyed
 * on the category when the asset is missing.
 *
 * v1 ships **synth-only** — no asset files (`asset` left undefined on
 * every entry). The synth fallback (§4.5 / Appendix A in the spec) is
 * intentionally small, zero-dependency, and polite. A CC0 asset pack
 * is a follow-up; dropping in `asset: "sounds/turn-complete.ogg"` on
 * an entry and shipping the file under `public/sounds/` is all it
 * takes to upgrade.
 */

export type SoundId =
    | "agent.turn.complete"
    | "agent.turn.error"
    | "agent.turn.interrupted"
    | "agent.message.accepted"
    | "agent.message.rejected";

export type SoundCategory = "success" | "info" | "warning" | "error";

export interface SoundDef {
    id: SoundId;
    /** User-visible label — surfaced by a future settings UI. */
    label: string;
    /** Drives the synth fallback's timbre when no asset is present. */
    category: SoundCategory;
    /** Path under `public/`. Optional — synth fallback covers absence. */
    asset?: string;
    /** Per-event enable key. Conventionally `notify:sound:<id>`. */
    settingKey: keyof SettingsType;
    /**
     * Coalesce window in ms — two firings of the same sound within this
     * window play once. Defends against double-dispatch storms during
     * tightly-coupled reducer event sequences. 300ms is generous for
     * "back-to-back legitimate events" but tight enough to drop the
     * common late-`stream-unsubscribed` race after `turn-ended`.
     */
    coalesceMs?: number;
}

export const SOUNDS: Record<SoundId, SoundDef> = {
    "agent.turn.complete": {
        id: "agent.turn.complete",
        label: "Agent turn completed",
        category: "success",
        settingKey: "notify:sound:agent.turn.complete",
        coalesceMs: 300,
    },
    "agent.turn.error": {
        id: "agent.turn.error",
        label: "Agent turn errored",
        category: "error",
        settingKey: "notify:sound:agent.turn.error",
        coalesceMs: 300,
    },
    "agent.turn.interrupted": {
        id: "agent.turn.interrupted",
        label: "Agent turn interrupted",
        category: "warning",
        settingKey: "notify:sound:agent.turn.interrupted",
        coalesceMs: 300,
    },
    "agent.message.accepted": {
        id: "agent.message.accepted",
        label: "Pending message accepted",
        category: "info",
        settingKey: "notify:sound:agent.message.accepted",
        coalesceMs: 150,
    },
    "agent.message.rejected": {
        id: "agent.message.rejected",
        label: "Pending message rejected",
        category: "warning",
        settingKey: "notify:sound:agent.message.rejected",
        coalesceMs: 150,
    },
};
