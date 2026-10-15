// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * Keep the rows on screen where they are when rows above them are removed or
 * inserted: a live-feed roll-off removing old turns, the history link
 * appearing at the top. docs/specs/SPEC_AGENT_PANE_NO_CATCH_UP_MOTION_2026_10_15.md §4.1.
 *
 * The layout slice prunes a removed row's height but keeps `scrollTop`, so the
 * next window was computed over shifted positions with the old scroll: it
 * selected rows about the removed height further down (or none), the rows on
 * screen unmounted, and the store only learned the browser's clamped scroll a
 * frame later. For a frame or two the visible history was missing, then
 * remounted: the flicker on the first send after a long turn.
 *
 * Before a change, `captureOnScreen` notes the rows on screen and how far the
 * viewport top is past each one's start. After it, `restoreOnScreen` finds the
 * first of them still there and puts the scroll position (the slice's and the
 * scroller's) at the same offset from its new start. Nothing is read from the
 * DOM: positions come from the slice's layout view.
 */

import {
    dispatchIfRegistered as dispatchLayoutIfRegistered,
    snapshot as snapshotLayout,
    type LayoutView,
} from "@/app/store/agent-pane-layout-store";

/** A row on screen, and how far the viewport top is past its start. */
export interface OnScreenRow {
    nodeId: string;
    offsetPx: number;
}

/** The rows on screen in `view` (not the overscan), top first, at most `max`. */
export function onScreenRows(
    view: LayoutView | null | undefined,
    scrollTop: number,
    viewportPx: number,
    max = 8
): OnScreenRow[] {
    const out: OnScreenRow[] = [];
    if (!view || viewportPx <= 0) return out;
    const bottom = scrollTop + viewportPx;
    for (let i = view.window.startIndex; i <= view.window.endIndex && out.length < max; i++) {
        const row = view.rows[i];
        if (!row || row.end <= scrollTop) continue;
        if (row.start >= bottom) break;
        out.push({ nodeId: row.nodeId, offsetPx: scrollTop - row.start });
    }
    return out;
}

/**
 * The scroll position that puts the first of `rows` still in `view` back at
 * its offset, or null when none is left or none moved. The rows are in
 * document order, and a removal or insertion keeps that order, so the first
 * one found is the topmost survivor.
 */
export function keptScrollTop(rows: readonly OnScreenRow[], view: LayoutView | null | undefined, scrollTop: number): number | null {
    if (!view || rows.length === 0) return null;
    const offsets = new Map(rows.map((r) => [r.nodeId, r.offsetPx] as const));
    for (const row of view.rows) {
        const offset = offsets.get(row.nodeId);
        if (offset === undefined) continue;
        const top = Math.max(0, row.start + offset);
        return Math.abs(top - scrollTop) < 0.5 ? null : top;
    }
    return null;
}

/** Before a change to the slice's rows: the rows on screen now. */
export function captureOnScreen(blockId: string, view: LayoutView | null | undefined): OnScreenRow[] {
    const snap = snapshotLayout(blockId);
    return snap ? onScreenRows(view, snap.scrollTop, snap.viewportPx) : [];
}

/** After it: keep them where they were, in the slice and in `scroller`. */
export function restoreOnScreen(
    blockId: string,
    rows: readonly OnScreenRow[],
    view: LayoutView | null | undefined,
    scroller: HTMLElement | undefined
): void {
    const snap = snapshotLayout(blockId);
    if (!snap) return;
    const top = keptScrollTop(rows, view, snap.scrollTop);
    if (top == null) return;
    if (scroller) scroller.scrollTop = top;
    dispatchLayoutIfRegistered(blockId, { type: "Scrolled", scrollTop: top, viewportPx: snap.viewportPx });
}
