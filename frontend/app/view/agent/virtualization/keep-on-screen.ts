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
import type { RowPosition } from "@/app/store/agent-pane-layout/reducer";
import { REATTACH_PX } from "@/app/scroll/follow-controller";
import type { DocumentNode } from "../../../agent-model/types";
import { scrollTranscriptTo } from "../scroll/one-way-flow";
import { firstHiddenIdOf } from "./verbosity";

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
 * Where the row `id` is now that the view changed what the list shows: a row a switch hid is in the
 * run that holds it; a run that went (its rows show again) is its first row, or the run that holds that.
 * Null when it is nowhere (the row left the document).
 */
export function successorRow(id: string, view: LayoutView, nodes: readonly DocumentNode[]): RowPosition | null {
    const rowOf = (rowId: string) => view.rows.find((r) => r.nodeId === rowId) ?? null;
    const first = firstHiddenIdOf(id);
    const target = first ?? id;
    const run = nodes.find((n) => n.type === "hidden_run" && n.hiddenIds.includes(target));
    return rowOf(target) ?? (run ? rowOf(run.id) : null);
}

/**
 * The scroll position that puts the topmost of `rows` back at its offset. The topmost row stands in
 * for itself; when the view hid it, the run that holds it does (the offset kept within the run's
 * height). Otherwise the first of `rows` still in `view`. Null when none is left or none moved. The
 * rows are in document order, and a removal or insertion keeps that order, so the first one found
 * is the topmost survivor.
 */
export function keptScrollTop(
    rows: readonly OnScreenRow[],
    view: LayoutView | null | undefined,
    scrollTop: number,
    nodes: readonly DocumentNode[] = []
): number | null {
    if (!view || rows.length === 0) return null;
    const settle = (row: RowPosition, offset: number) => {
        const top = Math.max(0, row.start + Math.min(offset, row.height));
        return Math.abs(top - scrollTop) < 0.5 ? null : top;
    };
    const offsets = new Map(rows.map((r) => [r.nodeId, r.offsetPx] as const));
    let held: RowPosition | null = null;
    for (const row of view.rows) {
        if (row.nodeId === rows[0].nodeId) return settle(row, rows[0].offsetPx);
        if (!held && offsets.has(row.nodeId)) held = row;
    }
    const successor = successorRow(rows[0].nodeId, view, nodes);
    if (successor) return settle(successor, rows[0].offsetPx);
    return held ? settle(held, offsets.get(held.nodeId)!) : null;
}

/**
 * Whether the viewport is at the bottom of the rows: a following pane is, and the follow flow keeps it
 * there. Restoring "the same offset from a row" would write the scroll position from row positions that
 * are still estimates (a table or a reply not measured yet), by however far the estimate is off, and the
 * follow controller then sees a scroll up it did not ask for.
 */
export function atBottomOf(view: LayoutView | null | undefined, scrollTop: number, viewportPx: number): boolean {
    return !!view && scrollTop + viewportPx >= view.totalSize - REATTACH_PX;
}

/** Before a change to the slice's rows: the rows on screen now (none at the bottom, see `atBottomOf`). */
export function captureOnScreen(blockId: string, view: LayoutView | null | undefined): OnScreenRow[] {
    const snap = snapshotLayout(blockId);
    if (!snap || atBottomOf(view, snap.scrollTop, snap.viewportPx)) return [];
    return onScreenRows(view, snap.scrollTop, snap.viewportPx);
}

/** After it: keep them where they were, in the slice and in `scroller`. */
export function restoreOnScreen(
    blockId: string,
    rows: readonly OnScreenRow[],
    view: LayoutView | null | undefined,
    scroller: HTMLElement | undefined,
    nodes: readonly DocumentNode[] = []
): void {
    const snap = snapshotLayout(blockId);
    if (!snap) return;
    const top = keptScrollTop(rows, view, snap.scrollTop, nodes);
    if (top == null) return;
    if (scroller) scrollTranscriptTo(scroller, top);
    dispatchLayoutIfRegistered(blockId, { type: "Scrolled", scrollTop: top, viewportPx: snap.viewportPx });
}
