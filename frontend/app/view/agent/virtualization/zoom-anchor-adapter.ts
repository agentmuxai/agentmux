// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The transcript's side of zoom that keeps the text under the pointer in place (`@/app/store/zoom-anchor`).
 * CSS zoom reflows the rows, so a ratio of the scroll is wrong; what is held is the text position
 * under the pointer, and after the change the scroll moves by how far that glyph moved. Measuring
 * where it is now means a pass never undoes a correction a live arrival already made. Rects are in
 * zoomed px, `scrollTop` in unzoomed px, so a move is divided by the zoom. An unmounted row is held
 * by its start in the layout slice until it is back. SPEC_ZOOM_ANCHOR_AND_AUDIT_2026_10_29.md §4.1.
 */

import { useZoomAnchor, type ZoomAnchorAdapter, type ZoomPoint } from "@/app/store/zoom-anchor";
import { elementOf, glyphRect, inside, textPositionAt, TOLERANCE_PX } from "@/app/store/zoom-anchor-dom";
import type { FollowScroll } from "../scroll/follow-scroll";
import { scrollTranscriptBy, scrollTranscriptTo } from "../scroll/one-way-flow";
import type { AgentDocumentVirtualListProps } from "./AgentDocumentVirtualList";

export interface ListZoomAnchorDeps {
    scroller: () => HTMLElement | undefined;
    /** The pane's zoom now: before the change when capturing, after it when restoring. */
    zoom: () => number;
    /** The pane follows its latest output (one writer then: the follow flow). */
    following: () => boolean;
    /** A row's start in the scroller, unzoomed px, from the layout slice; undefined when it has none. */
    rowStart: (nodeId: string) => number | undefined;
}

interface ListAnchor {
    /** The text under the pointer: a node and an offset in it. Null when the point was over no text. */
    node: Node | null;
    offset: number;
    /** The row it is in. */
    nodeId: string;
    /** Where in its glyph the pointer was: 0 at the top, 1 at the bottom. */
    glyphY: number;
    /** How far below the scroller's top the pointer was, in zoomed px. */
    viewportY: number;
    /** How far below its row's top the pointer was, in unzoomed px (stands in for an unmounted row). */
    rowOffsetPx: number;
}

/** The mounted row nearest the client y, for a point that is over no text (a gap, padding). */
function rowNear(scroller: HTMLElement, y: number): HTMLElement | null {
    let best: HTMLElement | null = null;
    let bestGap = Infinity;
    for (const row of scroller.querySelectorAll<HTMLElement>("[data-node-id]")) {
        const box = row.getBoundingClientRect();
        const gap = y < box.top ? box.top - y : y > box.bottom ? y - box.bottom : 0;
        if (gap < bestGap) {
            best = row;
            bestGap = gap;
        }
    }
    return best;
}

export function listZoomAnchor(deps: ListZoomAnchorDeps): ZoomAnchorAdapter<ListAnchor> {
    return {
        capture(point) {
            const scroller = deps.scroller();
            if (!scroller || deps.following()) return null;
            const box = scroller.getBoundingClientRect(); // perf:allow-layout-read — once per zoom gesture, before the change
            if (box.height <= 0) return null;
            const x = point && inside(box, point) ? point.clientX : box.left + box.width / 2;
            const y = point && inside(box, point) ? point.clientY : box.top + box.height / 2;

            // The engine answers with the nearest text when the point is over none (a gap, a spacer):
            // trust it only when that text is in a row and within a line of the point.
            const at = textPositionAt(x, y);
            const node = at && scroller.contains(at.node) ? at.node : null;
            const nodeRow = node && elementOf(node)?.closest<HTMLElement>("[data-node-id]");
            const glyph = nodeRow ? glyphRect(node!, at!.offset) : null;
            const onText = !!glyph && y >= glyph.top - glyph.height && y <= glyph.bottom + glyph.height;
            const row =
                (onText && nodeRow) ||
                (document.elementFromPoint(x, y) as HTMLElement | null)?.closest<HTMLElement>("[data-node-id]") ||
                rowNear(scroller, y);
            const nodeId = row?.dataset.nodeId;
            if (!row || !nodeId || !scroller.contains(row)) return null;

            const rowTop = row.getBoundingClientRect().top; // perf:allow-layout-read — once per zoom gesture, before the change
            return {
                node: onText ? node : null,
                offset: at?.offset ?? 0,
                nodeId,
                glyphY: onText && glyph!.height > 0 ? Math.min(1, Math.max(0, (y - glyph!.top) / glyph!.height)) : 0.5,
                viewportY: y - box.top,
                rowOffsetPx: (y - rowTop) / (deps.zoom() || 1),
            };
        },

        restore(anchor) {
            const scroller = deps.scroller();
            if (!scroller || deps.following()) return true;
            const zoom = deps.zoom() || 1;

            if (anchor.node?.isConnected && scroller.contains(anchor.node)) {
                const glyph = glyphRect(anchor.node, anchor.offset); // perf:allow-layout-read — settle passes, a few frames per zoom gesture
                if (glyph) {
                    const want = scroller.getBoundingClientRect().top + anchor.viewportY; // perf:allow-layout-read — settle passes, a few frames per zoom gesture
                    const have = glyph.top + anchor.glyphY * glyph.height;
                    if (Math.abs(have - want) < TOLERANCE_PX) return true;
                    scrollTranscriptBy(scroller, (have - want) / zoom);
                    return false;
                }
            }

            // The row is not mounted: where the slice says it starts.
            const start = deps.rowStart(anchor.nodeId);
            if (start === undefined) return true;
            const top = Math.max(0, start + anchor.rowOffsetPx - anchor.viewportY / zoom);
            if (Math.abs(top - scroller.scrollTop) * zoom >= TOLERANCE_PX) scrollTranscriptTo(scroller, top);
            return false;
        },
    };
}

/** Opt the transcript into zoom anchoring (a pane that follows is held by the follow flow instead). */
export function useListZoomAnchor(
    props: AgentDocumentVirtualListProps,
    scroller: () => HTMLElement | undefined,
    follow: () => FollowScroll | null
): void {
    if (!props.blockId) return;
    const deps: ListZoomAnchorDeps = {
        scroller,
        zoom: () => props.zoomFactor?.() ?? 1,
        following: () => (follow() ? follow()!.following() : props.viewState.stickToBottom()),
        rowStart: (nodeId) => props.layoutView?.()?.rows[props.viewState.indexOf(nodeId)]?.start,
    };
    useZoomAnchor({ blockId: props.blockId, zoom: deps.zoom, scroller, following: deps.following, adapter: listZoomAnchor(deps) });
}
