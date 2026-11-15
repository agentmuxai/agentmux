// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The asks that already ended, for two minutes (docs/specs/SPEC_STATUS_BAR_COMPACT_SUMMARY_AND_VAULT_REQUESTS_2026_11_15.md §4,
 * D6): the tone played, someone answered it in another window or it was closed, and the row is gone. The status bar's list
 * keeps them so "what was that sound?" has an answer. A short ring in this window's memory, read reactively; the registry
 * (waiting-for-you.ts) says when a pane's last wait ends, and nothing else is asked.
 *
 * Left out: an ask that waited under `ENDED_MIN_WAIT_MS` (answered at once, nobody went looking), and a Vault ask (the
 * Vault's own lists are the operator's, and a row here would show them to a widget).
 */

import { createSignal } from "solid-js";
import { attentionWho } from "./attention-list";
import { onWaitingEnded, type EndedWait } from "./waiting-for-you";
import { APP_KEY_PREFIX } from "./waiting-keys";

/** How long an ended ask is kept (ms). */
export const ENDED_WINDOW_MS = 120_000;
/** How many are kept. */
export const ENDED_RING = 10;
/** An ask that waited less than this is not recorded: it was answered before anyone could look for it (ms). */
export const ENDED_MIN_WAIT_MS = 5_000;

export interface EndedAsk {
    /** Unique within the ring. */
    key: string;
    blockId: string;
    /** Who asked: the agent's name, the pane's own name, or "AgentMux" for a request with no pane. */
    name: string;
    color?: string;
    what: string;
    /** When it began and when it ended (unix ms). */
    since: number;
    endedAt: number;
    /** `answered` when it was answered, `closed` when it went away unanswered. */
    how: "answered" | "closed";
}

const [ring, setRing] = createSignal<readonly EndedAsk[]>([]);
let counter = 0;

/** The asks that ended within the last two minutes, newest first; reactive to the ring, so a caller that wants the window to
 *  move on passes `now` from its own tick. */
export function endedAsks(now: number): EndedAsk[] {
    return ring()
        .filter((e) => now - e.endedAt < ENDED_WINDOW_MS)
        .slice()
        .reverse();
}

/** Note an ask that ended; one that waited too briefly, or a Vault ask, is not kept. */
export function noteEndedAsk(ask: Omit<EndedAsk, "key">): void {
    setRing((all) => [...all, { ...ask, key: `ended:${++counter}` }].slice(-ENDED_RING));
}

/** The registry's end of a pane's last wait, as an ended ask (or nothing). Exported for the test. */
export function endedAskOf(ended: EndedWait, endedAt: number): Omit<EndedAsk, "key"> | undefined {
    if (ended.kind === "vault" || endedAt - ended.since < ENDED_MIN_WAIT_MS || ended.since <= 0) return undefined;
    const who = ended.blockId.startsWith(APP_KEY_PREFIX) ? { name: "AgentMux", color: undefined } : attentionWho(ended.blockId);
    return {
        blockId: ended.blockId,
        name: who.name,
        color: who.color,
        what: ended.what,
        since: ended.since,
        endedAt,
        how: ended.reason === "closed" ? "closed" : "answered",
    };
}

let installed = false;

/** Keep the ring as asks end. Call once per window, after the registry is installed. */
export function installEndedAsks(): void {
    if (installed) return;
    installed = true;
    onWaitingEnded((ended) => {
        const ask = endedAskOf(ended, Date.now());
        if (ask) noteEndedAsk(ask);
    });
}

/** Test helper: forget what ended. */
export function __resetEndedAsks(): void {
    setRing([]);
}
