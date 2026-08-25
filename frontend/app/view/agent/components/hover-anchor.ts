// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * hover-anchor — finds the nearest scrolling ancestor of a hovered row, so
 * a hover-to-peek overlay can cap its own height to the space actually
 * available inside the pane rather than the whole viewport.
 *
 * Formerly also home to `pickExpandDirection`/`maxOverlayHeight`, the
 * above/below placement picker UserMessageBlock.tsx's "Session context"
 * overlay used before it (and every other hover-to-peek surface) migrated
 * onto PeekOverlay.tsx's Portal-rendered, top-anchored-only positioning —
 * see that file's doc comment for why a direction picker is no longer
 * needed. Removed as dead code once the last caller migrated off it.
 */

// Shared hover-to-peek enter delay. Was 150ms (separately declared in
// ToolBlock.tsx/MarkdownBlock.tsx/UserMessageBlock.tsx — three copies that
// could silently drift). Lowered to 50ms per SPEC_TRANSCRIPT_NODE_HOVER_PEEK_ALL_KINDS
// (2026-08-25): every node kind now gets a peek, so a snappier response
// matters more than the old ToolBlock-only guard against "accidental
// expansion during a fast scroll-through" — 50ms is still well above a
// single scroll-wheel tick's dwell time, just no longer padded for a
// feature that only existed on three anchor types.
export const PEEK_ENTER_DELAY_MS = 50;

export interface VerticalRect {
    /** Distance from viewport top to top edge, in px. */
    readonly top: number;
    /** Distance from viewport top to bottom edge, in px. */
    readonly bottom: number;
}

/**
 * Find the nearest scrolling ancestor of `el` (the element whose
 * computed `overflow-y` is `auto`, `scroll`, or `hidden`). Returns
 * its viewport-coordinate rect, or a viewport-wide fallback if
 * no such ancestor exists.
 *
 * Used by PeekOverlay.tsx to cap the hover-to-peek overlay's own
 * `max-height` to the space available inside the pane (typically
 * `.agent-document`, the agent-pane's scroll container) rather than the
 * whole viewport — a hovered row near the bottom of a split-view pane can
 * be far from the window's own bottom edge.
 */
export function findScrollContainerRect(el: HTMLElement): VerticalRect {
    let current: HTMLElement | null = el.parentElement;
    while (current && current !== document.body) {
        const cs = window.getComputedStyle(current);
        const oy = cs.overflowY;
        if (oy === "auto" || oy === "scroll" || oy === "hidden") {
            const r = current.getBoundingClientRect();
            return { top: r.top, bottom: r.bottom };
        }
        current = current.parentElement;
    }
    // No scroll container found — overlay can use the whole viewport.
    return { top: 0, bottom: window.innerHeight };
}
