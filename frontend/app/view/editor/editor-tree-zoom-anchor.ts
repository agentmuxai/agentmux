// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The file tree's side of zoom that keeps a row in place (`@/app/store/zoom-anchor`). What is held is a
 * row's path and how far below the scroller's top it was: the row under the pointer, else the selected
 * row when it is on screen (the keyboard has no pointer), else the row at the scroller's centre. The tree
 * is not virtualised, so the row is measured from the DOM. Rects are zoomed px and `scrollTop` is
 * unzoomed, so a move is divided by the zoom.
 */

import type { ZoomAnchorAdapter, ZoomPoint } from "@/app/store/zoom-anchor";
import { anchorPoint, inside, TOLERANCE_PX } from "@/app/store/zoom-anchor-dom";

interface RowAnchor {
    path: string;
    /** How far below the scroller's top the row's top was, zoomed px. */
    offsetY: number;
}

export interface TreeZoomAnchorDeps {
    scroller: () => HTMLElement | null | undefined;
    /** The tree's zoom now: after the change when restoring. */
    zoom: () => number;
}

const rowAt = (scroller: HTMLElement, x: number, y: number): HTMLElement | null =>
    document.elementFromPoint(x, y)?.closest<HTMLElement>("[role=treeitem][data-path]") ?? null;

/** The row to hold: under the pointer, else the selected one on screen, else the one at the middle. */
function pickRow(scroller: HTMLElement, box: DOMRect, point: ZoomPoint | null): HTMLElement | null {
    if (point && inside(box, point)) {
        const row = rowAt(scroller, point.clientX, point.clientY);
        if (row && scroller.contains(row)) return row;
    }
    const selected = scroller.querySelector<HTMLElement>("[role=treeitem][aria-selected=true]");
    if (selected) {
        const r = selected.getBoundingClientRect(); // perf:allow-layout-read — once per zoom gesture, before the change
        if (r.bottom > box.top && r.top < box.bottom) return selected;
    }
    const { x, y } = anchorPoint(box, null);
    return rowAt(scroller, x, y);
}

const rowOf = (scroller: HTMLElement, path: string): HTMLElement | null =>
    scroller.querySelector<HTMLElement>(`[role=treeitem][data-path="${CSS.escape(path)}"]`);

export function treeZoomAnchor(deps: TreeZoomAnchorDeps): ZoomAnchorAdapter<RowAnchor> {
    return {
        capture(point) {
            const scroller = deps.scroller();
            if (!scroller) return null;
            const box = scroller.getBoundingClientRect(); // perf:allow-layout-read — once per zoom gesture, before the change
            if (box.height <= 0) return null;
            const row = pickRow(scroller, box, point);
            const path = row?.dataset.path;
            if (!row || !path) return null;
            return { path, offsetY: row.getBoundingClientRect().top - box.top };
        },

        restore(anchor) {
            const scroller = deps.scroller();
            const row = scroller && rowOf(scroller, anchor.path);
            if (!scroller || !row) return true;
            const box = scroller.getBoundingClientRect(); // perf:allow-layout-read — settle passes, a few frames per zoom gesture
            const moved = row.getBoundingClientRect().top - box.top - anchor.offsetY;
            if (Math.abs(moved) < TOLERANCE_PX) return true;
            scroller.scrollTop += moved / (deps.zoom() || 1);
            return false;
        },
    };
}
