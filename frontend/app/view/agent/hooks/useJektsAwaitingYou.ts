// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The card for a sensitive jekt that waits on the person (jekts-awaiting-you.ts): one card in the pane's request
 * dock for the newest, naming how many older ones wait too, and a wait registered for the pane (the tone, the badge,
 * the notification) while any does. It goes when the person next sends in the pane or dismisses it.
 *
 * The card belongs to the pane, not to this mount: a tab switch unmounts the pane while the message still waits, so
 * the card is kept by pane in this module and a pane that mounts again finds it instead of raising a second.
 */

import { createEffect, createMemo, createSignal, untrack } from "solid-js";

import type { DocumentNode, JektMessageNode } from "@/app/agent-model/node-types";
import { META_AWAITING_CONFIRM } from "@/app/store/agent-status";
import { getBlockMetaKeyAtom } from "@/app/store/block-atom-cache";
import { paneRequests, type PaneRequestHandle } from "@/app/store/pane-requests";
import { JektNeedsYouCard } from "../components/JektNeedsYouCard";
import { syncAwaitingConfirm } from "./awaiting-user";
import { dismissalKey, jektsAwaitingYou, WINDOW_STARTED_MS } from "../jekts-awaiting-you";

/** A pane's card, and the messages it was raised for. */
const cards = new Map<string, { key: string; handle: PaneRequestHandle }>();
/** What the person dismissed, by pane, and a tick that says it changed (read by every mount). */
const dismissals = new Map<string, Set<string>>();
const [dismissedTick, bumpDismissed] = createSignal(0);
/** The jump of the pane's latest mount: the card outlives the mount that raised it. */
const jumpers = new Map<string, (nodeId: string) => void>();

export interface UseJektsAwaitingYouOptions {
    blockId: string;
    getDocument: () => readonly DocumentNode[];
    /** Scroll the transcript to a node. */
    jumpTo: (nodeId: string) => void;
}

export function useJektsAwaitingYou(opts: UseJektsAwaitingYouOptions): void {
    const { blockId } = opts;
    const dismissed = dismissals.get(blockId) ?? new Set<string>();
    dismissals.set(blockId, dismissed);
    jumpers.set(blockId, opts.jumpTo);
    // A flag left by a window that closed with the card up: nothing waits in this window, so it ends.
    const flagged = getBlockMetaKeyAtom(blockId, META_AWAITING_CONFIRM as never) as () => unknown;
    if (!cards.has(blockId) && flagged() === true) syncAwaitingConfirm(blockId, false);

    const waiting = createMemo(
        () => {
            dismissedTick();
            return jektsAwaitingYou(opts.getDocument(), dismissed, WINDOW_STARTED_MS);
        },
        [],
        { equals: (a, b) => a.length === b.length && a.every((n, i) => n.id === b[i].id) }
    );

    createEffect(() => {
        const list = waiting();
        untrack(() => {
            const key = list.map(dismissalKey).join("\0");
            const card = cards.get(blockId);
            if (card?.key === key) return;
            card?.handle.close();
            cards.delete(blockId);
            if (list.length === 0) {
                // The wait is over: the pane's "Waiting for you" (tab badge, Swarm) follows it.
                if (card) syncAwaitingConfirm(blockId, false);
                return;
            }
            const newest = list[list.length - 1];
            const handle = paneRequests.push(blockId, {
                source: "jekt-needs-you",
                Component: JektNeedsYouCard,
                props: {
                    jekt: newest,
                    earlier: list.length - 1,
                    onShow: (nodeId: string) => jumpers.get(blockId)?.(nodeId),
                    onDismiss: () => {
                        for (const n of list) dismissed.add(dismissalKey(n));
                        bumpDismissed((v) => v + 1);
                    },
                },
                text: `A sensitive message from ${newest.from} needs you`,
                summary: `${newest.from}: sensitive message`,
            });
            cards.set(blockId, { key, handle });
            syncAwaitingConfirm(blockId, true);
        });
    });
}

/**
 * The jekts that wait on the person in this pane right now, for another reader of the same wait (the Messages
 * panel's "Needs you" strip): the card's own list, so a Dismiss on either clears both. Tracks the dismissals.
 */
export function awaitingYou(blockId: string, nodes: readonly DocumentNode[]): JektMessageNode[] {
    dismissedTick();
    return jektsAwaitingYou(nodes, dismissals.get(blockId) ?? new Set<string>(), WINDOW_STARTED_MS);
}

/** Dismiss every message waiting in the pane: the card's Dismiss, from the panel. */
export function dismissAwaiting(blockId: string, waiting: readonly JektMessageNode[]): void {
    const dismissed = dismissals.get(blockId) ?? new Set<string>();
    dismissals.set(blockId, dismissed);
    for (const n of waiting) dismissed.add(dismissalKey(n));
    bumpDismissed((v) => v + 1);
}
