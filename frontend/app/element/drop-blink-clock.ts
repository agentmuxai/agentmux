// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The window's blink budget for the Pane Tab strip's drop-target hover.
 *
 * The hover blinks three times (drop-feedback.scss `$drop-blink-cycle` and
 * `$drop-blink-count`) and then lands in the held negative. WCAG 2.3.1 allows at
 * most three flashes in any one second, and the hover class is re-applied on every
 * entry, so a pointer wobbling over two panes would otherwise start a burst per
 * entry. One clock per window: a burst may start only when a second has passed
 * since the previous one began, anywhere in the window. A hover that arrives
 * inside that second skips the blink and lands inverted at once.
 * docs/specs/SPEC_PANE_TAB_MULTI_SELECT_MOVE_AND_COLLAPSED_EXPAND_2026_10_24.md §3.
 */

/** The shortest gap between the starts of two blink bursts. */
export const BLINK_WINDOW_MS = 1000;

let lastBurstAt = Number.NEGATIVE_INFINITY;

/** Whether a blink burst may start now. When it may, the clock records that it did. */
export function claimBlink(now: number = performance.now()): boolean {
    if (now - lastBurstAt < BLINK_WINDOW_MS) return false;
    lastBurstAt = now;
    return true;
}

/** For tests. */
export function resetBlinkClock(): void {
    lastBurstAt = Number.NEGATIVE_INFINITY;
}
