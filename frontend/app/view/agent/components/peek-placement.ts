// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * peek-placement — where `PeekOverlay`'s shrink-wrapped ("end") panel goes.
 *
 * Pure geometry, no DOM, so the one invariant that matters can be swept by a
 * test: **the pointer is never inside the panel.** The panel is portalled, so
 * the hovered row sees `mouseleave` the instant the pointer moves onto it; a
 * panel that covers the pointer closes, reopens under the pointer, and flickers
 * at the enter delay.
 *
 * The panel always sits NEAR THE POINTER, like every other peek: below it, or
 * above it, with `CURSOR_GAP_PX` of clearance. Horizontal and vertical are
 * decided separately.
 *
 * Horizontal (`computePeekHorizontal`)
 *  - Fits within the row's width: right edge on the row's right edge, growing
 *    leftward (what it always did).
 *  - Wider than the row: **pinned to the row's left edge** and extending right,
 *    over the pane border, up to the window edge. It uses as much width as
 *    exists, and never less than the row.
 *
 * Vertical (`computePeekVertical`)
 *  - Fits on one side of the pointer inside the transcript container: placed
 *    there (what it always did).
 *  - Otherwise it leaves the container, up to the window edge, on the side of
 *    the pointer with more room, and its HEIGHT is cut to that room so it can
 *    never reach the pointer; the rest scrolls inside it. The earlier code
 *    clamped the position instead and let the box grow back over the pointer.
 *
 * A panel that had to be cut (`scrolls`) is meant to be entered — its scroll
 * bar and text must be reachable — so `PeekOverlay` bridges the pointer from the
 * row to it.
 *
 * docs/reports/REPORT_TOOL_HOVER_PANEL_SIZE_AND_PLACEMENT_2026_10_02.md
 */

export interface PeekRect {
    left: number;
    right: number;
    top: number;
    bottom: number;
}

export interface PeekVerticalBounds {
    top: number;
    bottom: number;
}

/** Vertical clearance between the pointer and the panel. */
export const CURSOR_GAP_PX = 12;
/** Bottom margin inside the transcript container. */
export const BOTTOM_MARGIN_PX = 4;
/** Clearance kept from the window edge when the panel leaves the pane. */
export const VIEWPORT_MARGIN_PX = 8;

// ── Horizontal ───────────────────────────────────────────────────────────────

export interface PeekHorizontalInput {
    row: PeekRect;
    viewport: { width: number; height: number };
    /** The panel's max-content width, with no width cap. */
    naturalWidth: number;
}

export interface PeekHorizontal {
    /** The panel's left edge, or its RIGHT edge when `alignRight`. */
    left: number;
    maxWidth: number;
    /** True: `left` is the panel's right edge (apply `translateX(-100%)`). */
    alignRight: boolean;
    /** True when the panel reaches past the row's own edges. */
    extendsPastRow: boolean;
}

export function computePeekHorizontal(input: PeekHorizontalInput): PeekHorizontal {
    const { row, viewport, naturalWidth } = input;
    const rowWidth = row.right - row.left;
    const rightAligned: PeekHorizontal = {
        left: row.right,
        maxWidth: rowWidth,
        alignRight: true,
        extendsPastRow: false,
    };
    if (naturalWidth <= rowWidth) return rightAligned;
    // Wider than the row: pin the left edge and use the room to the window's
    // right edge. If that is no more than the row has, nothing is gained.
    const room = viewport.width - VIEWPORT_MARGIN_PX - row.left;
    if (room <= rowWidth) return rightAligned;
    return {
        left: row.left,
        maxWidth: room,
        alignRight: false,
        extendsPastRow: Math.min(naturalWidth, room) > rowWidth,
    };
}

// ── Vertical ─────────────────────────────────────────────────────────────────

export interface PeekVerticalInput {
    row: PeekRect;
    /** The pointer's Y, or null if it has not moved over the row yet. */
    mouseY: number | null;
    /** The transcript's scroll container. */
    container: PeekVerticalBounds;
    viewport: { width: number; height: number };
    /** The panel's height with no height cap, laid out at its final width. */
    naturalHeight: number;
}

export interface PeekVertical {
    top: number;
    maxHeight: number;
    /** True when the panel reaches beyond the transcript container. */
    leavesContainer: boolean;
    /** True when the height had to be cut, so the content scrolls inside. */
    scrolls: boolean;
}

export function computePeekVertical(input: PeekVerticalInput): PeekVertical {
    const { row, mouseY, container, viewport, naturalHeight } = input;
    const h = Math.max(0, naturalHeight);
    const windowBottom = viewport.height - VIEWPORT_MARGIN_PX;

    // No pointer position yet: at the row's top, as it always was; nothing to avoid.
    if (mouseY == null) {
        const room = Math.max(0, windowBottom - row.top);
        const maxHeight = Math.min(h, room);
        return {
            top: row.top,
            maxHeight,
            leavesContainer: row.top + maxHeight > container.bottom,
            scrolls: maxHeight < h,
        };
    }

    // Fits on one side of the pointer, inside the container.
    const belowTop = mouseY + CURSOR_GAP_PX;
    const containerLimit = container.bottom - BOTTOM_MARGIN_PX;
    if (belowTop + h <= containerLimit) {
        return { top: Math.max(belowTop, container.top), maxHeight: h, leavesContainer: false, scrolls: false };
    }
    const aboveTop = mouseY - CURSOR_GAP_PX - h;
    if (aboveTop >= container.top) {
        return { top: aboveTop, maxHeight: h, leavesContainer: false, scrolls: false };
    }

    // Leaves the container: the side of the pointer with more room, height cut
    // to that room so it cannot reach the pointer.
    const roomBelow = Math.max(0, windowBottom - belowTop);
    const roomAbove = Math.max(0, mouseY - CURSOR_GAP_PX - VIEWPORT_MARGIN_PX);
    const useBelow = h <= roomBelow || (h > roomAbove && roomBelow >= roomAbove);
    const maxHeight = Math.min(h, useBelow ? roomBelow : roomAbove);
    const top = useBelow ? belowTop : mouseY - CURSOR_GAP_PX - maxHeight;
    return {
        top,
        maxHeight,
        leavesContainer: top < container.top || top + maxHeight > container.bottom,
        scrolls: maxHeight < h,
    };
}

// ── For the invariant test ───────────────────────────────────────────────────

/** Vertical extent the placed panel occupies. */
export function placedExtent(v: PeekVertical): { top: number; bottom: number } {
    return { top: v.top, bottom: v.top + v.maxHeight };
}

/** Horizontal extent the placed panel occupies, for a panel of this natural width. */
export function placedSpan(hz: PeekHorizontal, naturalWidth: number): { left: number; right: number } {
    const w = Math.min(naturalWidth, hz.maxWidth);
    return hz.alignRight ? { left: hz.left - w, right: hz.left } : { left: hz.left, right: hz.left + w };
}
