// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The status bar's "last sound" and the Messages panel's "Open in Audit" open the Audit at one row: the pane shows
// the kind of row it is, finds the row once the list holds it, scrolls to it and keeps it lit for a few seconds.

import { createEffect, createMemo, createSignal, on, onCleanup, untrack, type Accessor } from "solid-js";

import type { AuditEntry } from "@/types/rpc/AuditEntry";
import { rowOfMessage, type MessageRef } from "./audit-message";
import { createAuditSounds, kindOfSound, MATCH_MS, rowOf, type SoundRef } from "./audit-sound";

/** How long the row stays lit once found. */
export const FOCUS_MS = 4000;
/** How long the list is given to hold the row (a kind that was off has to be fetched). */
const AWAIT_MS = 5000;

/** A row asked for: the sound the status bar played, or a message the Messages panel holds. */
export interface RowAsk {
    /** When it was asked: the same row asked for again is a new ask. */
    asked: number;
    /** When the row was written, about: the list is paged back until it reaches past this. */
    at: number;
    /** The kind of row, shown when the filter hides it. */
    kind?: string;
    find: (entries: readonly AuditEntry[]) => AuditEntry | undefined;
}

export const soundAsk = (ref: SoundRef | undefined): RowAsk | undefined =>
    ref && { asked: ref.asked, at: ref.at, kind: kindOfSound(ref.event), find: (entries) => rowOf(entries, ref) };

export const messageAsk = (ref: MessageRef | undefined): RowAsk | undefined =>
    ref && { asked: ref.asked, at: ref.at, kind: "jekt", find: (entries) => rowOfMessage(entries, ref.msgId) };

/** The later of two asks: a pane that was asked for a sound and then for a message follows the message. */
export const newestAsk = (a: RowAsk | undefined, b: RowAsk | undefined): RowAsk | undefined =>
    a && b ? (b.asked >= a.asked ? b : a) : (a ?? b);

/** A row asked for a moment ago, once per ask: the pane's meta changing in another key, or an ask left in
 *  the meta from before a restart, asks for nothing. */
const ASK_FRESH_MS = 10_000;

export function askedRow(read: Accessor<RowAsk | undefined>): Accessor<RowAsk | undefined> {
    return createMemo(
        () => {
            const ask = read();
            return ask && Date.now() - ask.asked <= ASK_FRESH_MS ? ask : undefined;
        },
        undefined,
        { equals: (a, b) => a?.asked === b?.asked && a?.at === b?.at }
    );
}

export function createRowFocus(opts: {
    request: Accessor<RowAsk | undefined>;
    entries: Accessor<readonly AuditEntry[]>;
    /** Show rows of this kind (it was filtered out). */
    showKind: (kind: string) => void;
    scrollTo: (rowId: string) => void;
    /** Page older entries in until `done()` (the row is in the list, or the list reaches back past the sound). */
    reachBack?: (done: () => boolean) => Promise<void>;
}): Accessor<string | undefined> {
    const [focused, setFocused] = createSignal<string | undefined>();
    let awaiting: { ask: RowAsk; until: number } | undefined;
    let reaching = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const look = () => {
        if (!awaiting) return;
        const row = awaiting.ask.find(untrack(opts.entries));
        if (!row) {
            // Paging older entries in takes as long as it takes.
            if (Date.now() > awaiting.until && !reaching) awaiting = undefined;
            return;
        }
        awaiting = undefined;
        setFocused(row.id);
        if (timer !== undefined) clearTimeout(timer);
        timer = setTimeout(() => setFocused(undefined), FOCUS_MS);
        // The row is drawn when the list changes; scroll once it is.
        queueMicrotask(() => opts.scrollTo(row.id));
    };

    // More than a page of entries can have been written since the sound: page back to it.
    const reach = (ask: RowAsk) => {
        reaching++;
        const done = () => {
            const entries = untrack(opts.entries);
            return ask.find(entries) !== undefined || entries.some((e) => e.ts_ms < ask.at - MATCH_MS);
        };
        void opts.reachBack!(done).finally(() => {
            reaching--;
            look();
        });
    };

    createEffect(
        on(opts.request, (ask) => {
            if (!ask) return;
            awaiting = { ask, until: Date.now() + AWAIT_MS };
            if (ask.kind) opts.showKind(ask.kind);
            look();
            // After this flush, so the list the page-ins start from is the newest page of the filter just set.
            if (opts.reachBack) queueMicrotask(() => reach(ask));
        })
    );
    createEffect(on(opts.entries, look, { defer: true }));
    onCleanup(() => timer !== undefined && clearTimeout(timer));
    return focused;
}

/** The Audit's sounds (speakers, the lit row, the last sound), with the row an ask names (the status bar's last
 *  sound, a message of the Messages panel) kept lit and scrolled to. `showKind` shows a kind of row the filter hides; `body` is the pane's scroller;
 *  `agentIn` names the agent in a pane, for the last sound's line. */
export function createAuditSoundsAt(
    entries: Accessor<readonly AuditEntry[]>,
    now: Accessor<number>,
    ask: Accessor<RowAsk | undefined>,
    pane: {
        showKind: (kind: string) => void;
        body: () => HTMLElement | undefined;
        reachBack?: (done: () => boolean) => Promise<void>;
        agentIn?: (blockId: string) => string | undefined;
    }
) {
    const focused = createRowFocus({
        request: askedRow(ask),
        entries,
        showKind: pane.showKind,
        reachBack: pane.reachBack,
        scrollTo: (id) => pane.body()?.querySelector(`[data-row-id="${id}"]`)?.scrollIntoView?.({ block: "center" }),
    });
    return createAuditSounds(entries, now, focused, pane.agentIn);
}
