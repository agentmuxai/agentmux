// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The window's flash budget for the Pane Tab strip's drop-target hover.
 *
 * WCAG 2.3.1 allows at most three flashes in any one second; a flash is a pair of
 * opposing luminance changes, so six changes. A hover that lands inverted makes
 * one change when it begins (normal to negative), two for each blink of its burst
 * (drop-feedback.scss `$drop-blink-count`), and one when it ends (negative to
 * normal): with two blinks, six, which is the whole budget. The hover class is
 * re-applied on every entry, so a pointer moving across panes would otherwise
 * stack bursts. Each entry that inverts is therefore charged its whole cost up
 * front, for the second that follows it, and an entry that does not fit gets the
 * outline pulse alone, without the negative:
 *
 *   blink   the burst, then held negative
 *   plain   the outline pulse only
 *
 * (Holding the negative without a blink would cost two more changes, which never
 * fits after a burst in the same second, so there is no third treatment.)
 * Panes are separate areas of the screen, so counting an exit on one pane against
 * a burst on another is the conservative reading.
 * docs/specs/SPEC_PANE_TAB_MULTI_SELECT_MOVE_AND_COLLAPSED_EXPAND_2026_10_24.md §3.
 */

/** Blinks in a burst. Must equal `$drop-blink-count` in drop-feedback.scss (a test checks). */
export const BLINK_COUNT = 2;

/** Luminance changes WCAG 2.3.1 allows in one second: three flashes of two changes. */
export const CHANGES_PER_SECOND = 6;

/** The sliding window the budget is counted over. */
export const FLASH_WINDOW_MS = 1000;

/** Changes a hover that inverts costs: enter, two per blink, leave. */
export const BLINK_COST = 2 + 2 * BLINK_COUNT;

export type HoverFlash = "blink" | "plain";

let charges: { at: number; cost: number }[] = [];

/** The treatment a hover entry at `now` gets. A blink's cost is recorded as spent. */
export function claimHoverFlash(now: number = performance.now()): HoverFlash {
    charges = charges.filter((c) => now - c.at < FLASH_WINDOW_MS);
    const spent = charges.reduce((sum, c) => sum + c.cost, 0);
    if (spent + BLINK_COST > CHANGES_PER_SECOND) return "plain";
    charges.push({ at: now, cost: BLINK_COST });
    return "blink";
}

/** For tests. */
export function resetFlashBudget(): void {
    charges = [];
}
