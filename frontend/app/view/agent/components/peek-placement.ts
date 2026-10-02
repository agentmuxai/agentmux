// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * peek-placement — where `PeekOverlay`'s shrink-wrapped ("end") panel goes.
 *
 * Pure geometry, no DOM, so the one invariant that matters can be swept by a
 * test: **the pointer is never inside the panel unless the panel was built to
 * be entered.** The panel is portalled, so the hovered row sees `mouseleave`
 * the instant the pointer moves onto it; a panel that covers the pointer
 * closes, reopens under the pointer, and flickers at the enter delay.
 *
 * Three modes, tried in this order:
 *
 *  1. `inside` — what it always was: right-aligned to the row, `CURSOR_GAP_PX`
 *     below the pointer (or above it), kept inside the transcript's scroll
 *     container. Used whenever the panel fits on one side of the pointer, so
 *     the common short peek looks exactly as before.
 *  2. `beside` — the panel is too tall for either side of the pointer. It goes
 *     next to the pane (right, else left), outside the row, so it cannot
 *     overlap the pointer whatever its height. It may extend past the pane, up
 *     to the window edge, and is capped to `PEEK_MAX_HEIGHT_VH` of the window
 *     with its own scroll bar. Because it is beside the row, the pointer can
 *     cross into it and use that scroll bar.
 *  3. `vertical` — no room beside the pane (it fills the window). Above or
 *     below the pointer, whichever has more room, with the height *constrained
 *     to that room*. The earlier code clamped the position instead and let the
 *     box grow back over the pointer.
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

export interface PeekPlacementInput {
    /** The hovered row, viewport px. */
    row: PeekRect;
    /** The pointer's Y, or null if it has not moved over the row yet. */
    mouseY: number | null;
    /** The transcript's scroll container. */
    container: PeekVerticalBounds;
    /** The window, viewport px. */
    viewport: { width: number; height: number };
    /**
     * The panel's height with no height cap, laid out at the row's width.
     * Measured at one fixed width so the choice of mode cannot depend on the
     * mode it chose last time (a wider `beside` panel is shorter, which would
     * otherwise flip it back to `inside`, and so on).
     */
    naturalHeight: number;
    /** The panel's height as currently rendered; used to keep `beside` on screen. */
    currentHeight: number;
}

export type PeekMode = "inside" | "beside" | "vertical";

export interface PeekPlacement {
    mode: PeekMode;
    left: number;
    top: number;
    maxWidth: number;
    maxHeight: number;
    /** True: `left` is the panel's RIGHT edge (apply `translateX(-100%)`). */
    alignRight: boolean;
}

/** Vertical clearance between the pointer and the panel (inside / vertical). */
export const CURSOR_GAP_PX = 12;
/** Bottom margin inside the transcript container. */
export const BOTTOM_MARGIN_PX = 4;
/** Clearance kept from the window edge when the panel leaves the pane. */
export const VIEWPORT_MARGIN_PX = 8;
/** Gap between the row's edge and a `beside` panel. */
export const SIDE_GAP_PX = 6;
/** A side must offer at least this much width to be used. */
export const MIN_SIDE_WIDTH_PX = 280;
/** A `beside` panel never gets wider than this, however much room there is. */
export const MAX_SIDE_WIDTH_PX = 720;
/** Tallest the panel gets before it scrolls: this share of the window height. */
export const PEEK_MAX_HEIGHT_VH = 0.6;

export function computePeekPlacement(input: PeekPlacementInput): PeekPlacement {
    const { row, mouseY, container, viewport, naturalHeight, currentHeight } = input;
    const rowWidth = row.right - row.left;
    const ceiling = Math.max(0, viewport.height * PEEK_MAX_HEIGHT_VH);
    const height = Math.min(naturalHeight, ceiling);

    // No pointer position yet: the panel sits at the row's top, as it always
    // did, and there is nothing to avoid.
    if (mouseY == null) {
        return {
            mode: "inside",
            left: row.right,
            top: row.top,
            maxWidth: rowWidth,
            maxHeight: Math.min(Math.max(0, container.bottom - row.top - BOTTOM_MARGIN_PX), ceiling),
            alignRight: true,
        };
    }

    // 1. inside: fits on one side of the pointer, within the container.
    const containerLimit = container.bottom - BOTTOM_MARGIN_PX;
    const belowTop = mouseY + CURSOR_GAP_PX;
    if (belowTop + height <= containerLimit) {
        const top = Math.max(belowTop, container.top);
        return inside(row, top, container, ceiling);
    }
    const aboveTop = mouseY - CURSOR_GAP_PX - height;
    if (aboveTop >= container.top) {
        return inside(row, aboveTop, container, ceiling);
    }

    // 2. beside: next to the pane, outside the row, so the pointer is not in it.
    const spaceRight = viewport.width - row.right - SIDE_GAP_PX - VIEWPORT_MARGIN_PX;
    const spaceLeft = row.left - SIDE_GAP_PX - VIEWPORT_MARGIN_PX;
    const rightOk = spaceRight >= MIN_SIDE_WIDTH_PX;
    const leftOk = spaceLeft >= MIN_SIDE_WIDTH_PX;
    if (rightOk || leftOk) {
        const useRight = rightOk && (!leftOk || spaceRight >= spaceLeft);
        const maxHeight = Math.min(Math.max(0, viewport.height - 2 * VIEWPORT_MARGIN_PX), ceiling);
        const shown = Math.min(currentHeight > 0 ? currentHeight : height, maxHeight);
        const wanted = Math.min(Math.max(row.top, container.top), container.bottom);
        const top = Math.max(VIEWPORT_MARGIN_PX, Math.min(wanted, viewport.height - VIEWPORT_MARGIN_PX - shown));
        return {
            mode: "beside",
            left: useRight ? row.right + SIDE_GAP_PX : row.left - SIDE_GAP_PX,
            top,
            maxWidth: Math.min(useRight ? spaceRight : spaceLeft, MAX_SIDE_WIDTH_PX),
            maxHeight,
            alignRight: !useRight,
        };
    }

    // 3. vertical: no side room. Constrain the HEIGHT to the larger side
    // rather than clamping the position back over the pointer.
    const spaceBelow = viewport.height - VIEWPORT_MARGIN_PX - belowTop;
    const spaceAbove = mouseY - CURSOR_GAP_PX - VIEWPORT_MARGIN_PX;
    const useBelow = spaceBelow >= spaceAbove;
    const room = Math.max(0, useBelow ? spaceBelow : spaceAbove);
    const maxHeight = Math.min(room, ceiling);
    const shown = Math.min(height, maxHeight);
    return {
        mode: "vertical",
        left: row.right,
        top: useBelow ? belowTop : mouseY - CURSOR_GAP_PX - shown,
        maxWidth: rowWidth,
        maxHeight,
        alignRight: true,
    };
}

function inside(row: PeekRect, top: number, container: PeekVerticalBounds, ceiling: number): PeekPlacement {
    return {
        mode: "inside",
        left: row.right,
        top,
        maxWidth: row.right - row.left,
        maxHeight: Math.min(Math.max(0, container.bottom - top - BOTTOM_MARGIN_PX), ceiling),
        alignRight: true,
    };
}

/**
 * The vertical extent the panel really occupies for a placement, given the
 * height it would have uncapped. Exported for the invariant test.
 */
export function placedExtent(p: PeekPlacement, naturalHeight: number): { top: number; bottom: number } {
    const h = Math.min(naturalHeight, p.maxHeight);
    return { top: p.top, bottom: p.top + h };
}

/** Horizontal extent of a placement for a panel of the given width. */
export function placedSpan(p: PeekPlacement, width: number): { left: number; right: number } {
    const w = Math.min(width, p.maxWidth);
    return p.alignRight ? { left: p.left - w, right: p.left } : { left: p.left, right: p.left + w };
}
