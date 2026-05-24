// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * hover-anchor — pure function for choosing the expansion direction
 * of a hover-expanded overlay so that the trigger (summary) stays
 * under the cursor across the expand transition.
 *
 * Used by `UserMessageBlock` for the startup-injection collapse.
 * The mechanic: at the moment hover fires (after the 150ms enter
 * delay), the body content is rendered absolutely above OR below
 * the summary based on viewport space. The summary itself never
 * moves in document flow — the body floats over neighbors.
 *
 * The direction picker considers BOTH:
 *   1. Whether the body fits below the summary without overflowing
 *      the viewport.
 *   2. Whether more space is available above or below the summary.
 *
 * Below wins on tie (matches the historical default of dropdowns,
 * and the keyboard-activated pinned-in-flow path also opens
 * downward, so the visual model is consistent).
 *
 * Spec:
 * `docs/specs/SPEC_STARTUP_HOVER_EXPANSION_ANCHOR_2026_05_24.md` §4.3.
 *
 * No dependency on `window` / DOM — caller passes in viewportHeight
 * so this stays a pure function and is trivially unit-testable.
 */

export type ExpandDirection = "above" | "below";

export interface VerticalRect {
    /** Distance from viewport top to top edge, in px. */
    readonly top: number;
    /** Distance from viewport top to bottom edge, in px. */
    readonly bottom: number;
}

// Backwards-compat alias — public API name from PR #1021 first cut.
// Both rects use the same shape (top + bottom in viewport coords).
export type SummaryRect = VerticalRect;

/**
 * Pick the side on which the floating body should render.
 *
 * Decision tree:
 *   1. If body fits below the summary inside the clipping
 *      container → "below".
 *   2. Else if body fits above → "above".
 *   3. Else pick whichever side has more room → "below" on a tie.
 *
 * Step 2 catches the "near-bottom" case — the canonical reason
 * the user asked for the direction-flip in the first place.
 *
 * The function is pure; pass exact values for unit testing. In
 * production the caller wires `summaryEl.getBoundingClientRect()`
 * and the nearest scrollable ancestor's rect (or
 * `{top: 0, bottom: window.innerHeight}` when no scroll
 * container exists).
 *
 * Why the container rect and not `window.innerHeight`: the agent
 * pane lives inside a scrollable `.agent-document` region. A
 * summary near the bottom of that pane can still be 200px above
 * the window's bottom — `window.innerHeight` would think there's
 * plenty of room and pick `below`, but the overlay would render
 * clipped or off the bottom of the pane. Codex P1 round 2 on
 * PR #1021.
 *
 * @param summaryRect  the summary's bounding rect in viewport
 *   coordinates.
 * @param containerRect  the nearest clipping ancestor's bounding
 *   rect, also in viewport coordinates. For the document body
 *   (no clipping), pass `{ top: 0, bottom: window.innerHeight }`.
 * @param bodyEstimate  the estimated rendered height of the body
 *   in CSS pixels. Conservative over-estimates are safe (they
 *   just push toward step 3's tie-break).
 */
export function pickExpandDirection(
    summaryRect: VerticalRect,
    containerRect: VerticalRect,
    bodyEstimate: number,
): ExpandDirection {
    const spaceBelow = Math.max(0, containerRect.bottom - summaryRect.bottom);
    const spaceAbove = Math.max(0, summaryRect.top - containerRect.top);

    // Step 1: body fits below — preferred direction.
    if (bodyEstimate <= spaceBelow) {
        return "below";
    }
    // Step 2: body doesn't fit below but fits above — flip.
    if (bodyEstimate <= spaceAbove) {
        return "above";
    }
    // Step 3: doesn't fit either way; pick the larger side. Below
    // wins on tie (and on the no-room-anywhere edge case where
    // both `spaceAbove` and `spaceBelow` are 0).
    return spaceBelow >= spaceAbove ? "below" : "above";
}

/**
 * Find the nearest scrolling ancestor of `el` (the element whose
 * computed `overflow-y` is `auto`, `scroll`, or `hidden`). Returns
 * its viewport-coordinate rect, or a viewport-wide fallback if
 * no such ancestor exists.
 *
 * Used by `UserMessageBlock` to feed `pickExpandDirection` the
 * right clipping bounds — typically `.agent-document` (the
 * agent-pane's scroll container).
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

/**
 * The pixel height the overlay should be capped at, given the
 * chosen direction and the clipping container's bounds. The
 * caller applies this as an inline `max-height` on the overlay
 * so that — for the "fits-neither" case — the overlay's own
 * `overflow-y: auto` activates inside the container's bounds
 * (instead of the overlay being clipped by the container and
 * the hidden tail being unreachable). Codex P2 round 2 on
 * PR #1021.
 *
 * `margin` is reserved space at the container edge so the
 * overlay doesn't sit flush against the pane border. 4px is
 * enough breathing room without compromising readable area.
 */
export function maxOverlayHeight(
    summaryRect: VerticalRect,
    containerRect: VerticalRect,
    direction: ExpandDirection,
    margin = 4,
): number {
    if (direction === "below") {
        return Math.max(0, containerRect.bottom - summaryRect.bottom - margin);
    }
    return Math.max(0, summaryRect.top - containerRect.top - margin);
}
