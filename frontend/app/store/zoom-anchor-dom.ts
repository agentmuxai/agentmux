// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * What the zoom anchors of the panes share (`./zoom-anchor`): finding the text under a point, the
 * rectangle of a character, and where a capture is taken (the pointer when it is over the scroller,
 * else its centre). Rects are in zoomed px and `scrollTop` in unzoomed px, so a move is divided by the
 * zoom. SPEC_ZOOM_ANCHOR_AND_AUDIT_2026_10_29.md §4.1 and §4.3.
 */

import type { ZoomPoint } from "./zoom-anchor";

type Doc = Document & {
    caretPositionFromPoint?: (x: number, y: number) => { offsetNode: Node; offset: number } | null;
    caretRangeFromPoint?: (x: number, y: number) => Range | null;
};

/** The text position under a client point, by whichever API this engine has. */
export function textPositionAt(x: number, y: number): { node: Node; offset: number } | null {
    const doc = document as Doc;
    if (doc.caretPositionFromPoint) {
        const p = doc.caretPositionFromPoint(x, y);
        return p ? { node: p.offsetNode, offset: p.offset } : null;
    }
    if (doc.caretRangeFromPoint) {
        const r = doc.caretRangeFromPoint(x, y);
        return r ? { node: r.startContainer, offset: r.startOffset } : null;
    }
    return null;
}

/**
 * The rectangle of the character at a text position: the one after it, else the one before it. A
 * collapsed range has no reliable box, and the same character at capture and restore keeps the two
 * measurements comparable. Null when it has no box (not laid out).
 */
export function glyphRect(node: Node, offset: number): DOMRect | null {
    const length = node.nodeType === Node.TEXT_NODE ? (node.textContent?.length ?? 0) : node.childNodes.length;
    const range = document.createRange();
    try {
        if (offset < length) {
            range.setStart(node, offset);
            range.setEnd(node, offset + 1);
        } else if (offset > 0) {
            range.setStart(node, offset - 1);
            range.setEnd(node, offset);
        } else {
            range.setStart(node, 0);
            range.setEnd(node, 0);
        }
    } catch {
        return null;
    }
    const rect = range.getClientRects()[0] ?? range.getBoundingClientRect();
    return rect && (rect.height > 0 || rect.width > 0) ? rect : null;
}

export const elementOf = (n: Node): Element | null =>
    n.nodeType === Node.ELEMENT_NODE ? (n as Element) : n.parentElement;

export const inside = (box: DOMRect, p: ZoomPoint): boolean =>
    p.clientX >= box.left && p.clientX <= box.right && p.clientY >= box.top && p.clientY <= box.bottom;

/** Pixels (zoomed) the held text may be off by and still count as in place. */
export const TOLERANCE_PX = 0.5;

/** The anchor point for a capture: the pointer when it is over the scroller, else the scroller's centre. */
export function anchorPoint(box: DOMRect, point: ZoomPoint | null): { x: number; y: number } {
    const over = point && inside(box, point);
    return { x: over ? point.clientX : box.left + box.width / 2, y: over ? point.clientY : box.top + box.height / 2 };
}
