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
 *    over the pane border, by at most half the row's width
 *    (`MAX_OVERSHOOT_FRACTION`: a 500px pane gives a panel up to 750px) and never
 *    past the window edge. Never less wide than the row.
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
 * row to it. It also PINS such a panel (stops recomputing from the live pointer
 * Y) and places it with `flushToRow`. Following the pointer made a tall panel
 * unreachable: every move toward it moved it away and cut its height again, and
 * the only way out crossed the next row, whose own peek then opened on top.
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
/** How far past its row's right edge a wide panel may reach, as a fraction of the row's width. */
export const MAX_OVERSHOOT_FRACTION = 0.5;

// ── Horizontal ───────────────────────────────────────────────────────────────

export interface PeekHorizontalInput {
    row: PeekRect;
    viewport: { width: number; height: number };
    /** The panel's max-content width, with no width cap. */
    naturalWidth: number;
    /**
     * The narrowest the panel may be (viewport px). It is placed as if it were at
     * least this wide, so a panel narrower than this but wider than its row goes
     * left-pinned and over the pane border like any wide panel. Capped to the room
     * actually available, so it never runs off the window.
     */
    minWidth?: number;
}

export interface PeekHorizontal {
    /** The panel's left edge, or its RIGHT edge when `alignRight`. */
    left: number;
    maxWidth: number;
    /** The min-width to apply: the requested minimum, capped to `maxWidth`. */
    minWidth: number;
    /** True: `left` is the panel's right edge (apply `translateX(-100%)`). */
    alignRight: boolean;
    /** True when the panel reaches past the row's own edges. */
    extendsPastRow: boolean;
}

export function computePeekHorizontal(input: PeekHorizontalInput): PeekHorizontal {
    const { row, viewport, naturalWidth } = input;
    const requestedMin = Math.max(0, input.minWidth ?? 0);
    // The width the panel will actually want: its content, or the minimum.
    const width = Math.max(naturalWidth, requestedMin);
    const rowWidth = row.right - row.left;
    const rightAligned: PeekHorizontal = {
        left: row.right,
        maxWidth: rowWidth,
        minWidth: Math.min(requestedMin, rowWidth),
        alignRight: true,
        extendsPastRow: false,
    };
    if (width <= rowWidth) return rightAligned;
    // Wider than the row: pin the left edge and extend right, by at most half the
    // row's width and never past the window's right edge. If that is no more
    // than the row has, nothing is gained.
    const room = Math.min(viewport.width - VIEWPORT_MARGIN_PX - row.left, rowWidth * (1 + MAX_OVERSHOOT_FRACTION));
    if (room <= rowWidth) return rightAligned;
    return {
        left: row.left,
        maxWidth: room,
        minWidth: Math.min(requestedMin, room),
        alignRight: false,
        extendsPastRow: Math.min(width, room) > rowWidth,
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
    /**
     * Keep the panel's near edge on the row: below the pointer it starts no lower
     * than the row's bottom, above it ends no higher than the row's top. For a
     * panel the pointer is meant to ENTER (`PeekOverlay` pins those): with the
     * plain `CURSOR_GAP_PX` offset there can be a strip of the NEXT row between
     * this row and the panel, and crossing it opens that row's peek on top of
     * this one. The pointer stays outside the panel either way, since it is on
     * the row.
     */
    flushToRow?: boolean;
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
    const { row, mouseY, container, viewport, naturalHeight, flushToRow } = input;
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

    // The panel's near edge on each side of the pointer (see `flushToRow`).
    const belowTop = flushToRow ? Math.min(mouseY + CURSOR_GAP_PX, row.bottom) : mouseY + CURSOR_GAP_PX;
    const aboveBottom = flushToRow ? Math.max(mouseY - CURSOR_GAP_PX, row.top) : mouseY - CURSOR_GAP_PX;

    // Fits on one side of the pointer, inside the container.
    const containerLimit = container.bottom - BOTTOM_MARGIN_PX;
    if (belowTop + h <= containerLimit) {
        return { top: Math.max(belowTop, container.top), maxHeight: h, leavesContainer: false, scrolls: false };
    }
    const aboveTop = aboveBottom - h;
    if (aboveTop >= container.top) {
        return { top: aboveTop, maxHeight: h, leavesContainer: false, scrolls: false };
    }

    // Leaves the container: the side of the pointer with more room, height cut
    // to that room so it cannot reach the pointer.
    const roomBelow = Math.max(0, windowBottom - belowTop);
    const roomAbove = Math.max(0, aboveBottom - VIEWPORT_MARGIN_PX);
    const useBelow = h <= roomBelow || (h > roomAbove && roomBelow >= roomAbove);
    const maxHeight = Math.min(h, useBelow ? roomBelow : roomAbove);
    const top = useBelow ? belowTop : aboveBottom - maxHeight;
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
    const w = Math.min(Math.max(naturalWidth, hz.minWidth), hz.maxWidth);
    return hz.alignRight ? { left: hz.left - w, right: hz.left } : { left: hz.left, right: hz.left + w };
}
