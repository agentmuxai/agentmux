// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The status bar's "last sound" opens the Audit at the row that sound came from: the pane shows the kind of row
// the sound is of, finds the row once the list holds it, scrolls to it and keeps it lit for a few seconds.

import { createEffect, createMemo, createSignal, on, onCleanup, untrack, type Accessor } from "solid-js";

import type { PlayedSound } from "@/app/notification/sound/sound-history";
import type { AuditEntry } from "@/types/rpc/AuditEntry";
import { createAuditSounds, kindOfSound, rowOf, type SoundRef } from "./audit-sound";

/** How long the row stays lit once found. */
export const FOCUS_MS = 4000;
/** How long the list is given to hold the row (a kind that was off has to be fetched). */
const AWAIT_MS = 5000;

/** A sound asked for a moment ago, once per ask: the pane's meta changing in another key, or an ask left in
 *  the meta from before a restart, asks for nothing. */
const ASK_FRESH_MS = 10_000;

export function askedSound(read: Accessor<SoundRef | undefined>): Accessor<SoundRef | undefined> {
    return createMemo(
        () => {
            const ask = read();
            return ask && Date.now() - ask.asked <= ASK_FRESH_MS ? ask : undefined;
        },
        undefined,
        { equals: (a, b) => a?.asked === b?.asked && a?.at === b?.at }
    );
}

export function createSoundFocus(opts: {
    request: Accessor<SoundRef | undefined>;
    entries: Accessor<readonly AuditEntry[]>;
    /** Show rows of this kind (it was filtered out). */
    showKind: (kind: string) => void;
    scrollTo: (rowId: string) => void;
}): Accessor<string | undefined> {
    const [focused, setFocused] = createSignal<string | undefined>();
    let awaiting: { sound: PlayedSound; until: number } | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const look = () => {
        if (!awaiting) return;
        const row = rowOf(untrack(opts.entries), awaiting.sound);
        if (!row) {
            if (Date.now() > awaiting.until) awaiting = undefined;
            return;
        }
        awaiting = undefined;
        setFocused(row.id);
        if (timer !== undefined) clearTimeout(timer);
        timer = setTimeout(() => setFocused(undefined), FOCUS_MS);
        // The row is drawn when the list changes; scroll once it is.
        queueMicrotask(() => opts.scrollTo(row.id));
    };

    createEffect(
        on(opts.request, (ask) => {
            if (!ask) return;
            awaiting = { sound: ask, until: Date.now() + AWAIT_MS };
            const kind = kindOfSound(ask.event);
            if (kind) opts.showKind(kind);
            look();
        })
    );
    createEffect(on(opts.entries, look, { defer: true }));
    onCleanup(() => timer !== undefined && clearTimeout(timer));
    return focused;
}

/** The Audit's sounds (speakers, the lit row, the last sound), with the row a status bar ask names kept lit and
 *  scrolled to. `showKind` shows a kind of row the filter hides; `body` is the pane's scroller. */
export function createAuditSoundsAt(
    entries: Accessor<readonly AuditEntry[]>,
    now: Accessor<number>,
    ask: Accessor<SoundRef | undefined>,
    pane: { showKind: (kind: string) => void; body: () => HTMLElement | undefined }
) {
    const focused = createSoundFocus({
        request: askedSound(ask),
        entries,
        showKind: pane.showKind,
        scrollTo: (id) => pane.body()?.querySelector(`[data-row-id="${id}"]`)?.scrollIntoView?.({ block: "center" }),
    });
    return createAuditSounds(entries, now, focused);
}
