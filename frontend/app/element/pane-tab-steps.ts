// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The steps a tab strip takes as its tabs run out of room (docs/reports/REPORT_PANE_TAB_STEPPED_SHRINK_2026_11_23.md).
 * One decision per strip, from the room each tab gets, so the tabs of a strip always look alike:
 *
 * - `full`: every tab keeps its icon, label and close button; labels are clipped with an ellipsis as the tabs share the
 *   squeeze (plain CSS shrink, no decision needed).
 * - `icon-close`: tabs that are not selected lose their label; the selected tab keeps it.
 * - `icon`: tabs that are not selected are the icon alone; the selected tab keeps its icon and close button.
 *
 * Below the smallest widths the strip stops shrinking and its header scrolls, as it always did. Pure: no DOM.
 */

export type PaneTabStep = "full" | "icon-close" | "icon";

/** Unzoomed px each tab gets, below which a step starts. */
export const NO_LABEL_BELOW_PX = 90;
export const ICON_ONLY_BELOW_PX = 48;
/** A step is left for a roomier one only once the share is this much past its threshold, so a strip sitting on a boundary does not flap. */
export const STEP_HYSTERESIS_PX = 8;

/** The least a tab that is not selected keeps, and the least the selected one keeps (icon, close button), in unzoomed px. */
export const RESTING_FLOOR_PX = 27;
/** A tab with no icon (a document tab) has only its label to say which it is: its floor leaves a few letters of it. */
export const RESTING_NO_ICON_FLOOR_PX = 56;
export const ACTIVE_FLOOR_PX = 47;

function rank(step: PaneTabStep): number {
    return step === "full" ? 0 : step === "icon-close" ? 1 : 2;
}

/** The step a share asks for with no memory of the last one. */
function target(share: number): PaneTabStep {
    if (share < ICON_ONLY_BELOW_PX) return "icon";
    if (share < NO_LABEL_BELOW_PX) return "icon-close";
    return "full";
}

/**
 * The step for a strip.
 * `share`: the strip's room for tabs divided by its tab count, in unzoomed px. `fits`: the tabs fit at their own widths, so
 * nothing needs to give. `previous`: the step it was in, for the hysteresis.
 */
export function stepFor(share: number, fits: boolean, previous: PaneTabStep): PaneTabStep {
    if (fits || !Number.isFinite(share)) return "full";
    const next = target(share);
    if (rank(next) >= rank(previous)) return next;
    // Going to a roomier step: only with margin past the threshold it left.
    return target(share - STEP_HYSTERESIS_PX);
}

/** The least room, in unzoomed px, the tabs need before the strip stops shrinking and its header scrolls instead. */
export function minTabsRoom(count: number, restingFloor: number = RESTING_FLOOR_PX): number {
    if (count <= 0) return 0;
    return ACTIVE_FLOOR_PX + (count - 1) * restingFloor;
}
