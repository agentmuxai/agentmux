// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The steps a tab strip takes as its tabs run out of room (docs/reports/REPORT_PANE_TAB_STEPPED_SHRINK_2026_11_23.md).
 * One decision per strip, from the room each tab gets, so the tabs of a strip always look alike:
 *
 * - `full`: every tab keeps its icon, label and close button; labels are clipped with an ellipsis as the tabs share the
 *   squeeze (plain CSS shrink, no decision needed).
 * - `icon-close`: tabs that are not selected lose their label; the selected tab keeps it.
 * - `icon`: tabs that are not selected are the icon alone; the selected tab keeps its icon, the start of its title and close button.
 *
 * Below the smallest widths the strip stops shrinking and its header scrolls, as it always did. Pure: no DOM.
 *
 * The rule for who gives way: the selected tab gets its room first (its icon, its own buttons and close button whole, and a
 * readable part of its title), and the other tabs give up width before it does, down to their floors. So the step is
 * decided by what the other tabs are left with once the selected tab has its room (`othersShare`), not by an equal share.
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
/** The part of the selected tab's title it keeps before anything of its icon, buttons or close button goes: about eight letters. */
export const ACTIVE_LABEL_MIN_PX = 56;
/** The most of its title the selected tab asks for (its label's own cap, PaneTabStrip.scss). */
export const ACTIVE_LABEL_MAX_PX = 190;
/** The selected tab with no icon: the resting floor's few letters of its label, plus its close button. */
export const ACTIVE_NO_ICON_FLOOR_PX = 76;

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

/**
 * What the selected tab keeps at the least, in unzoomed px: what is not its title (`fixed`: icon, buttons, close button, padding)
 * and the readable start of its title, or all of a title that is shorter (`label`).
 */
export function activeFloor(fixed: number, label: number): number {
    return Math.max(ACTIVE_FLOOR_PX, fixed + Math.min(label, ACTIVE_LABEL_MIN_PX));
}

/** What the selected tab takes when nothing is squeezed: its fixed part and its title, to the title's cap. */
export function activeWant(fixed: number, label: number): number {
    return Math.max(ACTIVE_FLOOR_PX, fixed + Math.min(label, ACTIVE_LABEL_MAX_PX));
}

/** What each tab that is not selected is left with: the room less the selected tab's, shared by the others. */
export function othersShare(room: number, count: number, want: number): number {
    if (count <= 1) return room;
    return Math.max(0, room - want) / (count - 1);
}

/**
 * The least room, in unzoomed px, the tabs need before the strip stops shrinking and its header scrolls instead:
 * the selected tab's floor (`activeFloor`: its icon and close button at the least) and the others'.
 */
export function minTabsRoom(
    count: number,
    restingFloor: number = RESTING_FLOOR_PX,
    activeFloor: number = ACTIVE_FLOOR_PX
): number {
    if (count <= 0) return 0;
    return Math.max(activeFloor, ACTIVE_FLOOR_PX) + (count - 1) * restingFloor;
}
