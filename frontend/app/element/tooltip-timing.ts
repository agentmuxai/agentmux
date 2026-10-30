// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// When a hover tooltip shows and goes, shared by the Tooltip component and the tooltip
// that stands in for native `title=` labels (title-tooltip.ts), so the two feel alike
// and never show together. Hovers are near-instant (operator, 2026-10-29).

/** A short rest on an anchor before the first tooltip shows, so sweeping across a toolbar does not flash every one. */
export const SHOW_DELAY_MS = 40;
/** How long a tooltip lingers after the pointer leaves, so crossing the gap between two items does not flicker. */
export const HIDE_DELAY_MS = 100;
/** After one tooltip closes, the next one within this time shows at once. */
export const WARM_MS = 400;

export interface OpenTip {
    /** Close now, without the lingering grace. */
    close: () => void;
}

let lastClosedAt = Number.NEGATIVE_INFINITY;
let openTip: OpenTip | null = null;

/** Another tooltip is open or has just closed, so the pointer is moving along a row of them. */
export function isWarm(self?: OpenTip): boolean {
    return (openTip !== null && openTip !== self) || performance.now() - lastClosedAt < WARM_MS;
}

/** `tip` is opening: close the one that was open, so two are never shown together. */
export function claimOpen(tip: OpenTip): void {
    if (openTip && openTip !== tip) openTip.close();
    openTip = tip;
}

/** `tip` has closed (or is going away). */
export function releaseOpen(tip: OpenTip): void {
    lastClosedAt = performance.now();
    if (openTip === tip) openTip = null;
}

/** Forget `tip` without counting a close, e.g. when its component is disposed. */
export function forgetOpen(tip: OpenTip): void {
    if (openTip === tip) openTip = null;
}

/** For tests: forget every open tooltip and the last close, so nothing counts as warm. */
export function resetTooltipTiming(): void {
    openTip = null;
    lastClosedAt = Number.NEGATIVE_INFINITY;
}
