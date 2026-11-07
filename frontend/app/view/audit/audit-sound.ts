// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Which rows make a sound, which row a sound that just played came from, and the "last sound" line
// (docs/specs/SPEC_AUDIT_LIVE_LOG_OF_EVERY_AGENT_ACTION_2026_11_03.md §3.3). A sound needs no row of its own: every
// one is caused by something already in the log, and the Audit finds the row by agent, kind and time.

import { createMemo, createSignal, onCleanup, type Accessor } from "solid-js";

import { recentSounds, subscribePlayedSounds, type PlayedSound } from "@/app/notification/sound/sound-history";
import { SOUNDS } from "@/app/notification/sound/sounds";
import { getSettingsKeyAtom } from "@/app/store/global";
import { ageMs, formatAge } from "@/app/view/warden-shared/warden-shared";
import type { AuditEntry } from "@/types/rpc/AuditEntry";
import { isFailure } from "./audit-shared";

/** How long the row a sound came from stays lit. */
export const LIT_MS = 3000;
/** A row is the one a sound came from if it was written within this long of it. */
export const MATCH_MS = 2000;
/** How long after it played a sound is still "the last sound". */
export const LAST_SOUND_MS = 60_000;

export type RowSound = PlayedSound["event"];

/** The sound a row makes when sounds are on: the same mapping the sound service uses, one place for the Audit. */
export function soundFor(entry: AuditEntry): RowSound | null {
    switch (entry.kind) {
        case "tool":
            return "tool";
        case "turn":
            return entry.outcome === "success" ? "agent.turn.complete" : "agent.turn.error";
        case "jekt": {
            // The sender's half makes none: the receiver's pane takes the message, or refuses it.
            if (entry.provenance?.sent_only) return null;
            if (isFailure(entry)) return "agent.message.rejected";
            return !entry.outcome || entry.outcome === "delivered" ? "agent.message.accepted" : null;
        }
        default:
            return null;
    }
}

/** Whether the person's settings let `sound` play: a muted one shows no badge. Reactive when read in a computation. */
export function soundIsOn(sound: RowSound): boolean {
    if (getSettingsKeyAtom("notify:sounds:enabled")() === false) return false;
    if (sound === "tool") return getSettingsKeyAtom("notify:tooltones:enabled")() !== false;
    return getSettingsKeyAtom(SOUNDS[sound].settingKey)() !== false;
}

/** The pane of the agent a row is about: a jekt's is its receiver's. */
function blockOf(entry: AuditEntry): string | undefined {
    return (entry.kind === "jekt" ? entry.to?.links.pane?.block_id : entry.links.pane?.block_id) || undefined;
}

/** The newest row `sound` came from: the same pane, the same kind of sound, written within [`MATCH_MS`] of it. */
export function rowOf(entries: readonly AuditEntry[], sound: PlayedSound): AuditEntry | undefined {
    return entries.find((e) => blockOf(e) === sound.blockId && soundFor(e) === sound.event && Math.abs(e.ts_ms - sound.at) <= MATCH_MS);
}

/** The row to light now: the one the newest sound played in the last [`LIT_MS`] came from. */
export function litRowId(entries: readonly AuditEntry[], sounds: readonly PlayedSound[], now: number): string | undefined {
    for (const sound of [...sounds].reverse()) {
        if (now - sound.at >= LIT_MS) break;
        const row = rowOf(entries, sound);
        if (row) return row.id;
    }
    return undefined;
}

/** What the last sound was, for the line "♪ agent · Bash · 4s ago", and the pane to reveal; none after a minute. */
export function lastSoundLine(entries: readonly AuditEntry[], sounds: readonly PlayedSound[], now: number): { text: string; blockId: string } | null {
    const sound = sounds.at(-1);
    if (!sound || now - sound.at > LAST_SOUND_MS) return null;
    const row = rowOf(entries, sound);
    const who = row ? (row.kind === "jekt" ? row.to?.agent_id : row.agent_id) : undefined;
    const what = sound.tool ?? SOUNDS[sound.event as keyof typeof SOUNDS]?.label ?? sound.event;
    return { text: `♪ ${[who, what, `${formatAge(ageMs(sound.at, now))} ago`].filter(Boolean).join(" · ")}`, blockId: sound.blockId };
}

/** What this window played, as a signal for as long as the caller lives. */
function usePlayedSounds(): Accessor<readonly PlayedSound[]> {
    const [sounds, setSounds] = createSignal(recentSounds());
    onCleanup(subscribePlayedSounds(() => setSounds(recentSounds())));
    return sounds;
}

/** The Audit's reading of what played: which rows show a speaker, which one is lit, and the last sound's line. */
export function createAuditSounds(entries: Accessor<readonly AuditEntry[]>, now: Accessor<number>) {
    const badge = (entry: AuditEntry): boolean => {
        const sound = soundFor(entry);
        return sound !== null && soundIsOn(sound);
    };
    const sounds = usePlayedSounds();
    const lit = createMemo(() => litRowId(entries(), sounds(), now()));
    const last = createMemo(() => lastSoundLine(entries(), sounds(), now()));
    return { badge, lit, last };
}
