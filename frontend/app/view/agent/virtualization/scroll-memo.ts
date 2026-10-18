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
import { registerRecycleBlocker } from "@/app/store/recycle-blockers";
import { readSession, writeSession } from "../reload-persistence";

const KEY = "agentmux:scroll:";

export type ScrollMemo = { bottom: true } | { nodeId: string; offsetPx: number };

const capturers = new Map<string, () => ScrollMemo | null>();

/** Panes that loaded history older than their restore read, this page. */
const olderLoaded = new Set<string>();

/** `useHistoryPagination` loaded an older page into `blockId`'s pane. */
export function noteOlderHistoryLoaded(blockId: string): void {
    olderLoaded.add(blockId);
}

/** How long, and for how many frames at most, a restored position is held while the rows settle. */
const RESTORE_HOLD_MS = 1000;
const RESTORE_HOLD_FRAMES = 60;

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
 * away): the first row that starts on screen, and how far below the top it
 * starts (a negative offset). Anchoring on the row cut by the top edge instead
 * broke when that row came back a different height (a tall tool output, in the
 * lab: 1411 px into it landed in a later row); a row that starts on screen is
 * placed by its start, whatever its height. Only when one row covers the whole
 * screen is that row used, with the offset into it. Stored unzoomed, like
 * `scrollTop`: `getBoundingClientRect` is in zoomed px under the pane's CSS
 * zoom, so it is divided by `zoom`.
 */
export function captureScrollMemo(scroller: HTMLElement | undefined, following: boolean, zoom = 1): ScrollMemo | null {
    if (following) return { bottom: true };
    if (!scroller) return null;
    const view = scroller.getBoundingClientRect();
    const scale = zoom > 0 ? zoom : 1;
    let cut: { nodeId: string; offsetPx: number } | null = null;
    for (const row of scroller.querySelectorAll<HTMLElement>("[data-node-id]")) {
        const box = row.getBoundingClientRect();
        const nodeId = row.dataset.nodeId;
        if (!nodeId || box.bottom <= view.top + 1) continue;
        if (box.top >= view.top - 1) {
            // Starts on screen (below the top edge, above the bottom one).
            if (box.top < view.bottom) return { nodeId, offsetPx: (view.top - box.top) / scale };
            break;
        }
        cut ??= { nodeId, offsetPx: (view.top - box.top) / scale };
    }
    return cut;
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
    /** The pane's CSS zoom (1 without one). */
    zoom?: Accessor<number>;
    historyReady: Accessor<boolean>;
    scrollToRow: (nodeId: string, offsetPx: number) => void;
}): void {
    const { blockId } = opts;
    if (!blockId) return;
    const capture = (): ScrollMemo | null =>
        captureScrollMemo(opts.scroller(), untrack(opts.following), opts.zoom ? untrack(opts.zoom) : 1);
    capturers.set(blockId, capture);
    onCleanup(() => {
        if (capturers.get(blockId) === capture) capturers.delete(blockId);
    });
    // A reload reads only the tail, so a pane scrolled up into pages it loaded
    // later would come back at the bottom.
    onCleanup(
        registerRecycleBlocker(() =>
            olderLoaded.has(blockId) && !untrack(opts.following) ? "an agent pane is scrolled up into older history" : null
        )
    );
    let restored = false;
    createEffect(() => {
        if (restored || !opts.historyReady()) return;
        restored = true;
        const memo = takeScrollMemo(blockId);
        if (!memo || "bottom" in memo) return; // the default: following, at the bottom
        // After the restored rows' first layout, so the row has a position; then
        // on each frame for a while: the rows above it are measured over the next
        // frames, and a pane that no longer follows keeps nothing in place, so the
        // row drifted (458 px in the lab). Input in the pane ends it at once.
        const scroller = opts.scroller();
        let userMoved = false;
        const stop = (): void => {
            userMoved = true;
        };
        const types = ["wheel", "pointerdown", "keydown", "touchstart"] as const;
        for (const t of types) scroller?.addEventListener(t, stop, { passive: true, once: true });
        const until = performance.now() + RESTORE_HOLD_MS;
        let frames = 0;
        const hold = (): void => {
            if (userMoved) return;
            opts.scrollToRow(memo.nodeId, memo.offsetPx);
            if (++frames < RESTORE_HOLD_FRAMES && performance.now() < until) requestAnimationFrame(hold);
            else for (const t of types) scroller?.removeEventListener(t, stop);
        };
        requestAnimationFrame(hold);
    });
}
