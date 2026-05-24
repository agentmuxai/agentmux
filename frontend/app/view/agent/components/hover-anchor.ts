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

export interface SummaryRect {
    /** Distance from viewport top to summary's top edge, in px. */
    readonly top: number;
    /** Distance from viewport top to summary's bottom edge, in px. */
    readonly bottom: number;
}

/**
 * Pick the side on which the floating body should render.
 *
 * Decision tree:
 *   1. If body fits below the summary inside the viewport → "below".
 *   2. Else if body fits above the summary inside the viewport → "above".
 *   3. Else pick whichever side has more room → "below" on a tie.
 *
 * Step 2 catches the "near-bottom" case — the canonical reason the
 * user asked for the direction-flip in the first place. Step 3
 * gracefully handles the (rare) case where the body is taller than
 * either side of the viewport; the body will need a scrollbar
 * regardless, but we pick the side that minimizes the scroll
 * surface.
 *
 * The function is pure; pass exact values for unit testing. In
 * production the caller wires `summaryEl.getBoundingClientRect()`,
 * `window.innerHeight`, and the body's estimated height.
 *
 * @param summaryRect  the summary's bounding rect in viewport
 *   coordinates (call sites use `getBoundingClientRect()`).
 * @param viewportHeight  the viewport height in CSS pixels
 *   (`window.innerHeight`).
 * @param bodyEstimate  the estimated rendered height of the body
 *   in CSS pixels. Caller derives this from
 *   `estimateUnwrappedTextHeight` or a constant for the startup
 *   payload. Conservative over-estimates are safe (they just
 *   push toward step 3's tie-break).
 */
export function pickExpandDirection(
    summaryRect: SummaryRect,
    viewportHeight: number,
    bodyEstimate: number,
): ExpandDirection {
    const spaceBelow = Math.max(0, viewportHeight - summaryRect.bottom);
    const spaceAbove = Math.max(0, summaryRect.top);

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
