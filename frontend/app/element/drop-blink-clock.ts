// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The window's flash budget for the Pane Tab strip's drop-target hover.
 *
 * WCAG 2.3.1 allows at most three flashes in any one second; a flash is a pair of
 * opposing luminance changes, so six changes. A hover that lands inverted makes
 * one change when it begins (normal to negative), two for each blink of its burst
 * (drop-feedback.scss `$drop-blink-count`), and one when it ends (negative to
 * normal). The hover class is re-applied on every entry, so a pointer moving
 * across panes would otherwise stack bursts, and the change on leaving one pane
 * lands right beside the changes of entering the next.
 *
 * Every change is therefore accounted for at its real time: a hover's entry and
 * blink changes are recorded when it begins, its exit is pending while it is open
 * (counted as if it were about to happen) and recorded at its real time when it
 * ends. A new hover gets the richest treatment whose whole cost fits in what the
 * last second left over:
 *
 *   blink   the burst, then held negative   (entry + 2 per blink + exit = 6)
 *   steady  held negative, no blink         (entry + exit = 2)
 *   plain   the outline pulse only          (0)
 *
 * Panes are separate areas of the screen, so counting an exit on one pane against
 * a burst on another is the conservative reading.
 *
 * A header small enough to fall under WCAG 2.3.1's general flash threshold is outside
 * the rule, and is outside this budget: it always gets the burst. The threshold is a
 * flashing area of more than a quarter of a 10-degree field of view: a quarter of
 * 341 by 256 px on the 1024 by 768 reference screen, 21,824 px² (W3C technique G176).
 * The pane the pointer left changes in the same frame as the one it enters, so each
 * gets half of that. Without this a pointer moving from pane to pane
 * (the exit of the one it left counts against the burst of the one it enters) got the
 * blink on the first pane and only the held negative, or the outline alone, on the
 * next ones, so only some panes seemed to flash.
 * docs/specs/SPEC_PANE_TAB_MULTI_SELECT_MOVE_AND_COLLAPSED_EXPAND_2026_10_24.md §3.
 */

/** A flashing area (CSS px²) at or under which a hover is outside the flash rule: half of the threshold,
 *  which is a quarter of a 341 by 256 px field (21,824 px²), because the pane the pointer left changes in
 *  the same frame as the one it enters. */
export const EXEMPT_AREA_PX2 = (341 * 256) / 4 / 2;

/** The pane header's edge glow: a strip is `min(EDGE_STRIP_SHARE of the header's width, EDGE_STRIP_PX)`,
 *  at the header's own zoom, and the header is `HEADER_PX` tall at zoom 1. Must equal
 *  `$drop-edge-strip-px` and `$drop-edge-strip-share` in drop-feedback.scss and `--header-height`
 *  (theme.scss); a test checks the first two. */
export const EDGE_STRIP_PX = 40;
export const EDGE_STRIP_SHARE = 0.25;
export const HEADER_PX = 33;

/**
 * The area (CSS px²) that flashes when a pane header's edge glow blinks: its two strips, not the
 * header. `width` and `height` are the header's rendered size, which carries its zoom, so the
 * strip's 40 px scale with `height / HEADER_PX`.
 */
export function edgeFlashArea(width: number, height: number): number {
    const strip = Math.min(width * EDGE_STRIP_SHARE, EDGE_STRIP_PX * (height / HEADER_PX));
    return 2 * strip * height;
}

/** Blinks in a burst. Must equal `$drop-blink-count` in drop-feedback.scss (a test checks). */
export const BLINK_COUNT = 2;

/** One phase of a blink: half of `$drop-blink-cycle` (a test checks). */
export const BLINK_PHASE_MS = 100;

/** Luminance changes WCAG 2.3.1 allows in one second: three flashes of two changes. */
export const CHANGES_PER_SECOND = 6;

/** The sliding window the budget is counted over. */
export const FLASH_WINDOW_MS = 1000;

/** Changes a hover that blinks costs in all: entering, two per blink, leaving. */
export const BLINK_COST = 2 + 2 * BLINK_COUNT;

/** Changes a hover that holds the negative without blinking costs: entering and leaving. */
export const STEADY_COST = 2;

export type HoverFlash = "blink" | "steady" | "plain";

export interface Hover {
    kind: HoverFlash;
    /** The pointer left, or the tab dropped. Safe to call more than once. */
    end: (now?: number) => void;
}

/** Timestamps of luminance changes made or already scheduled (a burst's are in the near future). */
let changes: number[] = [];
/** Hovers that have inverted and not ended: each has one change still to come. */
let pendingExits = 0;

function usedAt(now: number): number {
    changes = changes.filter((t) => t > now - FLASH_WINDOW_MS);
    return changes.length + pendingExits;
}

/** A hover starts at `now`. Its treatment is the richest that fits the budget; call `end` when it ends. */
export function beginHover(now: number = performance.now(), areaPx2?: number): Hover {
    // Too small an area to count as a flash: the burst, uncounted.
    if (areaPx2 != null && areaPx2 > 0 && areaPx2 <= EXEMPT_AREA_PX2) return { kind: "blink", end: () => {} };
    const used = usedAt(now);
    const kind: HoverFlash =
        used + BLINK_COST <= CHANGES_PER_SECOND
            ? "blink"
            : used + STEADY_COST <= CHANGES_PER_SECOND
              ? "steady"
              : "plain";
    if (kind !== "plain") {
        changes.push(now);
        if (kind === "blink") {
            for (let i = 1; i <= 2 * BLINK_COUNT; i++) changes.push(now + i * BLINK_PHASE_MS);
        }
        pendingExits++;
    }
    let ended = false;
    return {
        kind,
        end: (at: number = performance.now()) => {
            if (ended) return;
            ended = true;
            if (kind === "plain") return;
            pendingExits--;
            changes.push(at);
        },
    };
}

/** For tests. */
export function resetFlashBudget(): void {
    changes = [];
    pendingExits = 0;
}
