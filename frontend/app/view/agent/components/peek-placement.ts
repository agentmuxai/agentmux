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
 *  - Wider than the row: it reaches past the row, over the pane borders, by at
 *    most half the row's width in total (`MAX_OVERSHOOT_FRACTION`: a 500px pane
 *    gives a panel up to 750px), and never further than it takes to reach a
 *    readable width (`readableWidth`, about 120 characters of code): a pane
 *    already that wide gets no overshoot at all. That overshoot is split by where the row sits
 *    in the window: a pane at the left edge overshoots only to the right, one at
 *    the right edge only to the left, a centred one by a quarter on each side,
 *    and anything in between by interpolation. Room the window edge takes from
 *    one side goes to the other. Never less wide than the row.
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
/** How far past its row's edges a wide panel may reach in total, as a fraction of the row's width. */
export const MAX_OVERSHOOT_FRACTION = 0.5;
/**
 * The readable width the overshoot reaches for, in characters of the panel's
 * code font (`PeekOverlay` measures it). Code style guides settle on 80–120
 * characters; past that, width adds blank space and lines too long to track.
 * Why this and not a fraction of the window:
 * docs/reports/REPORT_TOOL_HOVER_PANEL_SIZE_AND_PLACEMENT_2026_10_02.md §9.
 */
export const READABLE_WIDTH_CHARS = 120;

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
    /**
     * The readable width (viewport px): `READABLE_WIDTH_CHARS` of the panel's
     * code font. The overshoot stops where the panel reaches it, so a pane at
     * least this wide gets none. Absent: no such limit.
     */
    readableWidth?: number;
}

export interface PeekHorizontal {
    /**
     * The anchor: the panel's left edge after it is moved left by `shift` of its
     * own width (`translateX(-shift * 100%)`).
     */
    left: number;
    maxWidth: number;
    /** The min-width to apply: the requested minimum, capped to `maxWidth`. */
    minWidth: number;
    /**
     * How much of its own width the panel is moved left of `left`, 0 to 1.
     * 1 puts its right edge on `left` (right-aligned to the row). Done in CSS so
     * the overshoot keeps its split whatever width the panel ends up.
     */
    shift: number;
    /** True when the panel reaches past the row's own edges. */
    extendsPastRow: boolean;
}

/** Where the row sits across the window: 0 at the left edge, 0.5 centred, 1 at the right. */
function rowSide(row: PeekRect, viewport: { width: number }): number {
    const free = viewport.width - (row.right - row.left);
    return free > 0 ? Math.min(1, Math.max(0, row.left / free)) : 0;
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
        shift: 1,
        extendsPastRow: false,
    };
    if (width <= rowWidth) return rightAligned;

    // Wider than the row: overshoot by at most half the row's width, and only
    // as far as the readable width, split by the row's place in the window,
    // inside the window's margins (or the row's own edges, if it already
    // reaches past them).
    const readable = input.readableWidth != null && input.readableWidth > 0 ? input.readableWidth : Infinity;
    const overshoot = Math.min(rowWidth * MAX_OVERSHOOT_FRACTION, Math.max(0, readable - rowWidth));
    const roomLeft = Math.max(0, row.left - VIEWPORT_MARGIN_PX);
    const roomRight = Math.max(0, viewport.width - VIEWPORT_MARGIN_PX - row.right);
    let toLeft = overshoot * rowSide(row, viewport);
    let toRight = overshoot - toLeft;
    // What one window edge takes goes to the other side.
    if (toLeft > roomLeft) {
        toRight = Math.min(roomRight, toRight + toLeft - roomLeft);
        toLeft = roomLeft;
    }
    if (toRight > roomRight) {
        toLeft = Math.min(roomLeft, toLeft + toRight - roomRight);
        toRight = roomRight;
    }
    const extra = toLeft + toRight;
    // No more room than the row has: nothing is gained.
    if (extra <= 0) return rightAligned;
    const room = rowWidth + extra;
    // The panel grows out of the row in the same proportion as the room does,
    // so it always covers the row and never leaves the room.
    const shift = toLeft / extra;
    return {
        left: row.left - toLeft + shift * room,
        maxWidth: room,
        minWidth: Math.min(requestedMin, room),
        shift,
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
    const left = hz.left - hz.shift * w;
    return { left, right: left + w };
}
