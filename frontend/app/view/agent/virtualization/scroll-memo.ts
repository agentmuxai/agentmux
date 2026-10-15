// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * Where each agent pane was scrolled, kept across a reload of its window.
 * docs/specs/SPEC_RENDERER_RECYCLE_WHEN_IDLE_2026_10_13.md §2 (scroll position).
 *
 * Recorded when the page goes away (`pagehide`, which a reload fires; also
 * `saveScrollMemos`, for the renderer recycle): "at the bottom" for a pane that
 * follows, else the row at the top of the screen and how far the viewport top
 * is past its start. Rows have stable ids (built from their transcript line),
 * so the same row is found after the reload. Restored once, after the pane's
 * history loads: if that row was loaded, it goes back to the same offset;
 * otherwise the pane stays at the bottom. Kept in sessionStorage, like the
 * composer's drafts (../reload-persistence.ts): a reload of the same window,
 * not an app restart.
 */

import { createEffect, onCleanup, untrack, type Accessor } from "solid-js";
import { readSession, writeSession } from "../reload-persistence";

const KEY = "agentmux:scroll:";

export type ScrollMemo = { bottom: true } | { nodeId: string; offsetPx: number };

const capturers = new Map<string, () => ScrollMemo | null>();

/** Record every mounted pane's position now. */
export function saveScrollMemos(): void {
    for (const [blockId, capture] of capturers) writeSession(KEY + blockId, capture() ?? undefined);
}

if (typeof window !== "undefined") window.addEventListener("pagehide", saveScrollMemos);

/** The position `blockId` had when the page went away, once; then forgotten. */
export function takeScrollMemo(blockId: string): ScrollMemo | undefined {
    const memo = readSession<ScrollMemo>(KEY + blockId);
    writeSession(KEY + blockId, undefined);
    if (!memo || typeof memo !== "object") return undefined;
    if ("bottom" in memo) return { bottom: true };
    return typeof memo.nodeId === "string" && typeof memo.offsetPx === "number" ? memo : undefined;
}

/**
 * The pane's position, read from the DOM (a one-time read, when the page goes
 * away): the first row whose bottom is below the scroller's top.
 */
export function captureScrollMemo(scroller: HTMLElement | undefined, following: boolean): ScrollMemo | null {
    if (following) return { bottom: true };
    if (!scroller) return null;
    const top = scroller.getBoundingClientRect().top;
    for (const row of scroller.querySelectorAll<HTMLElement>("[data-node-id]")) {
        const box = row.getBoundingClientRect();
        if (box.bottom > top + 1) {
            const nodeId = row.dataset.nodeId;
            return nodeId ? { nodeId, offsetPx: top - box.top } : null;
        }
    }
    return null;
}

/**
 * Record this pane's position when the page goes away, and put it back once
 * its history has loaded (`scrollToRow`: the list's scrollToNode with an
 * offset, which also stops following). Call in the list's setup.
 */
export function useScrollMemo(opts: {
    blockId: string | undefined;
    scroller: () => HTMLElement | undefined;
    following: Accessor<boolean>;
    historyReady: Accessor<boolean>;
    scrollToRow: (nodeId: string, offsetPx: number) => void;
}): void {
    const { blockId } = opts;
    if (!blockId) return;
    const capture = (): ScrollMemo | null => captureScrollMemo(opts.scroller(), untrack(opts.following));
    capturers.set(blockId, capture);
    onCleanup(() => {
        if (capturers.get(blockId) === capture) capturers.delete(blockId);
    });
    let restored = false;
    createEffect(() => {
        if (restored || !opts.historyReady()) return;
        restored = true;
        const memo = takeScrollMemo(blockId);
        if (!memo || "bottom" in memo) return; // the default: following, at the bottom
        // After the restored rows' first layout, so the row has a position.
        requestAnimationFrame(() => opts.scrollToRow(memo.nodeId, memo.offsetPx));
    });
}
