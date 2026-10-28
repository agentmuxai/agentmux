// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Where the toast stack sits so it never covers a pane's or drawer's primary actions.
//
// A pane or drawer marks the block that holds its actions (a composer and its Send button, a drawer's
// note field and Accept / Send back row) with `data-toast-avoid`. The stack starts bottom-right; when
// that would cover a marked block it moves up to sit just above it. Nothing marked in the way: it stays
// where it was. (With no room above a block the stack stays put: the top of the window is the same
// column, so it would cover the same block.)

/** Put this attribute on an element a toast must not cover. */
export const TOAST_AVOID_ATTR = "data-toast-avoid";

export interface Box {
    left: number;
    top: number;
    right: number;
    bottom: number;
}

export interface Placement {
    /** The stack's distance from the window's bottom edge. */
    bottom: number;
}

export interface PlacementInput {
    viewportWidth: number;
    viewportHeight: number;
    /** The stack's width and height. */
    width: number;
    height: number;
    /** The stack's gap from the window's right edge. */
    right: number;
    /** Its usual gap from the bottom edge (it clears the status bar). */
    bottom: number;
    /** The least gap kept from the window's top edge when it moves up. */
    top: number;
    /** The blocks to keep clear (empty or off-screen ones are ignored). */
    avoid: readonly Box[];
}

/** Space left between the stack and the block it clears. */
const GAP = 8;

const overlaps = (a: Box, b: Box) => a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;

export function placeToasts(p: PlacementInput): Placement {
    const left = p.viewportWidth - p.right - p.width;
    const stackAt = (top: number): Box => ({ left, right: left + p.width, top, bottom: top + p.height });
    const live = p.avoid.filter(
        (b) => b.right > b.left && b.bottom > b.top && b.bottom > 0 && b.top < p.viewportHeight && b.right > 0 && b.left < p.viewportWidth,
    );
    const hits = (top: number) => live.filter((b) => overlaps(stackAt(top), b));

    // 1. Where it always sits.
    const usualTop = p.viewportHeight - p.bottom - p.height;
    if (hits(usualTop).length === 0) return { bottom: p.bottom };

    // 2. Above the blocks it would cover (raising it can bring a higher block into the way, so repeat).
    let top = usualTop;
    for (let i = 0; i < live.length + 1; i++) {
        const blocking = hits(top);
        if (blocking.length === 0) break;
        top = Math.min(...blocking.map((b) => b.top)) - GAP - p.height;
    }
    if (top >= p.top && hits(top).length === 0) return { bottom: p.viewportHeight - top - p.height };

    // No room above: the usual place.
    return { bottom: p.bottom };
}
