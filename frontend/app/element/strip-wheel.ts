// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// How far the mouse wheel scrolls a tab strip (pane tabs, document tabs): a fixed distance per
// notch, however many pixels the platform reports for one. A trackpad's small deltas move the strip
// in proportion instead.

/** Pixels the strip moves per mouse-wheel notch; tune it here. */
export const STRIP_WHEEL_NOTCH_PX = 10;
/** A mouse wheel reports about this many pixels (100, or 120 on some systems) for one notch. */
const WHEEL_NOTCH_DELTA = 100;
/** A pixel delta from this size up is a wheel notch; smaller ones are a trackpad's. */
const NOTCH_MIN_DELTA = 50;

export function stripWheelDelta(deltaY: number, deltaMode = 0): number {
    const size = Math.abs(deltaY);
    if (size === 0) return 0;
    const sign = deltaY < 0 ? -1 : 1;
    // Line mode counts lines, three to a notch.
    if (deltaMode === 1) return sign * STRIP_WHEEL_NOTCH_PX * Math.max(1, Math.round(size / 3));
    if (size < NOTCH_MIN_DELTA) return deltaY * (STRIP_WHEEL_NOTCH_PX / WHEEL_NOTCH_DELTA);
    return sign * STRIP_WHEEL_NOTCH_PX * Math.max(1, Math.round(size / WHEEL_NOTCH_DELTA));
}
