// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether a window-tab switch is being painted, and the A/B flag for the switch's fast path.
 *
 * A switch holds from the click until the incoming tab has been painted (two frames on for a tab that swaps at the
 * click, the end of its reveal otherwise), and never longer than HOLD_CAP_MS, so nothing that waits for it waits long.
 * What waits: the flushes of hidden panes (stream-scheduler.ts), which dirty layout in tabs that are kept laid out.
 */

/** A hold is let go at this age whatever else happens. */
export const HOLD_CAP_MS = 1000;

let holdToken = 0;
let held = false;
let capTimer: ReturnType<typeof setTimeout> | null = null;

/**
 * The fast tab switch: a cold tab whose content is settled swaps at the click, hidden panes' flushes wait for the
 * switch's paint, and the layout read is skipped for a tab already laid out. Off unless `globalThis.__fastTabSwitch`
 * is true: the switch an A/B run flips on one build (a measured default follows the run).
 */
export function fastTabSwitch(): boolean {
    return (globalThis as { __fastTabSwitch?: unknown }).__fastTabSwitch === true;
}

/** Starts holding for a switch; answers the token its own `releaseSwitchPaint` needs. A newer switch re-arms the hold. */
export function holdForSwitchPaint(): number {
    held = true;
    const token = ++holdToken;
    if (capTimer !== null) clearTimeout(capTimer);
    capTimer = setTimeout(() => releaseSwitchPaint(token), HOLD_CAP_MS);
    return token;
}

/** Ends the hold of switch `token`; a newer switch's hold is left alone. */
export function releaseSwitchPaint(token: number): void {
    if (!held || token !== holdToken) return;
    held = false;
    if (capTimer !== null) {
        clearTimeout(capTimer);
        capTimer = null;
    }
}

export function switchPaintHeld(): boolean {
    return held;
}

/** Test seam. */
export function resetSwitchPaintHold(): void {
    held = false;
    holdToken = 0;
    if (capTimer !== null) clearTimeout(capTimer);
    capTimer = null;
}
