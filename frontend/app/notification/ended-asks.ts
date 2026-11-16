// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The asks that already ended, for two minutes (docs/specs/SPEC_STATUS_BAR_COMPACT_SUMMARY_AND_VAULT_REQUESTS_2026_11_15.md §4,
 * D6): the tone played, someone answered it in another window or it was closed, and the row is gone. The status bar's list
 * keeps them so "what was that sound?" has an answer. A short ring in this window's memory, read reactively; the registry
 * (waiting-for-you.ts) says when a pane's last wait ends, and nothing else is asked.
 *
 * Two ways an ask is seen to end: the registry's end of this window's own asks, and, for an agent of another window that
 * only srv's rows report, the pane dropping out of the list of what waits.
 *
 * Left out: an ask that waited under `ENDED_MIN_WAIT_MS` (answered at once, nobody went looking), and a Vault ask (the
 * Vault's own lists are the operator's, and a row here would show them to a widget).
 */

import { createEffect, createRoot, createSignal } from "solid-js";
import { sharedAgentRuntime } from "@/app/store/agent-runtime-live";
import { attentionItems, attentionWho, UNKNOWN_NAME, type AttentionItem } from "./attention-list";
import { onWaitingEnded, waitingPanes, type EndedWait } from "./waiting-for-you";
import { APP_KEY_PREFIX } from "./waiting-keys";

/** How long an ended ask is kept (ms). */
export const ENDED_WINDOW_MS = 120_000;
/** How many are kept. */
export const ENDED_RING = 10;
/** Two records of the same pane's end within this are one ask: the registry and srv's row both saw it (ms). A second ask of
 *  the pane must itself have waited `ENDED_MIN_WAIT_MS`, so it never ends this soon after the first. */
const SAME_END_MS = 3_000;
/** An ask that waited less than this is not recorded: it was answered before anyone could look for it (ms). */
const ENDED_MIN_WAIT_MS = 5_000;

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
    if (ring().some((e) => e.blockId === ask.blockId && Math.abs(ask.endedAt - e.endedAt) < SAME_END_MS)) return;
    setRing((all) => [...all, { ...ask, key: `ended:${++counter}` }].slice(-ENDED_RING));
}

/** The registry's end of a pane's last wait, as an ended ask (or nothing). Exported for the test. */
export function endedAskOf(ended: EndedWait, endedAt: number): Omit<EndedAsk, "key"> | undefined {
    if (ended.kind === "vault" || endedAt - ended.since < ENDED_MIN_WAIT_MS || ended.since <= 0) return undefined;
    return endedAskFrom(ended, endedAt, whoEnded(ended.blockId));
}

/** Who asked, as the window knows it now; a pane already closed has lost its row and its name, so what the list showed while it
 *  waited is used then. */
function whoEnded(blockId: string): { name: string; color?: string } {
    if (blockId.startsWith(APP_KEY_PREFIX)) return { name: "AgentMux", color: undefined };
    const who = attentionWho(blockId);
    return who.name === UNKNOWN_NAME ? (names.get(blockId) ?? who) : who;
}

function endedAskFrom(ended: EndedWait, endedAt: number, who: { name: string; color?: string }): Omit<EndedAsk, "key"> {
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

/** Who each pane that waits is, as the list named it while it waited: the pane may be gone when its wait ends. */
const names = new Map<string, { name: string; color?: string }>();

/** An ask that only srv's runtime rows report (an agent of another window), as it was when last seen waiting. */
interface RuntimeAsk {
    name: string;
    color?: string;
    what: string;
    since: number;
}
const runtimeOnly = new Map<string, RuntimeAsk>();
/** Panes whose ask this window's registry already ended while srv's row still lists them waiting: the row clearing later is
 *  not a second ask. Kept until the pane is no longer listed. */
const settled = new Set<string>();
const NONE: ReadonlySet<string> = new Set();

/**
 * One look at the list of everything that waits (`items`), given which panes this window's registry has (`registry`) and
 * which srv's rows still say wait (`srvWaiting`, before any Settings filter). The registry says when its own asks end; an
 * ask only srv's rows report (an agent of another window) leaves no end there, so it ends here: when its pane stops being
 * listed, the registry has no wait for it, and srv's row no longer says it waits (a Browser wait left out in Settings is
 * unlisted too, but not over). Exported for the test.
 */
export function noteAttention(items: AttentionItem[], registry: ReadonlySet<string>, now: number, srvWaiting: ReadonlySet<string> = NONE): void {
    const listed = new Set<string>();
    for (const i of items) {
        listed.add(i.blockId);
        names.set(i.blockId, { name: i.name, color: i.color });
        if (registry.has(i.blockId)) {
            runtimeOnly.delete(i.blockId);
            settled.delete(i.blockId);
        } else if (!settled.has(i.blockId)) {
            runtimeOnly.set(i.blockId, { name: i.name, color: i.color, what: i.what, since: i.since });
        }
    }
    for (const id of [...settled]) if (!listed.has(id)) settled.delete(id);
    for (const [blockId, ask] of [...runtimeOnly]) {
        if (listed.has(blockId) || srvWaiting.has(blockId)) continue;
        runtimeOnly.delete(blockId);
        names.delete(blockId);
        // The registry took it over, or it began and ended before anyone could look for it.
        if (registry.has(blockId) || now - ask.since < ENDED_MIN_WAIT_MS || ask.since <= 0) continue;
        noteEndedAsk({ blockId, name: ask.name, color: ask.color, what: ask.what, since: ask.since, endedAt: now, how: "answered" });
    }
}

/** The panes srv's runtime rows say wait on the person, as the rows are, whatever Settings leaves out of the list. */
function srvWaitingPanes(): Set<string> {
    const out = new Set<string>();
    for (const r of sharedAgentRuntime().rows()) {
        const id = r.state === "waiting_on_user" ? (r.waiting_in ?? r.block_id) : undefined;
        if (id) out.add(id);
    }
    return out;
}

let installed = false;

/** Keep the ring as asks end. Call once per window, after the registry is installed. */
export function installEndedAsks(): void {
    if (installed) return;
    installed = true;
    onWaitingEnded((ended) => {
        const ask = endedAskOf(ended, Date.now());
        runtimeOnly.delete(ended.blockId);
        names.delete(ended.blockId);
        settled.add(ended.blockId);
        if (ask) noteEndedAsk(ask);
    });
    createRoot(() =>
        createEffect(() => {
            const items = attentionItems();
            noteAttention(items, new Set(waitingPanes().map((p) => p.blockId)), Date.now(), srvWaitingPanes());
        })
    );
}

/** Test helper: forget what ended. */
export function __resetEndedAsks(): void {
    setRing([]);
    names.clear();
    runtimeOnly.clear();
    settled.clear();
}
