// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether the picture of a pane about to be dragged is still being taken (tearoff-snapshot.ts). The picture is
 * a capture of the window as it is drawn when the capture runs, which is some 50 to 250 ms after the press,
 * and a quick drag starts inside that time. Whatever the drag draws over the pane by then is in the picture
 * the user then holds: on 2026-11-10 the operator held a blurred pane, because the dragged tile's blur
 * (tilelayout.scss `.dragging`) was on screen before the capture ran. So the drag's own drawing (that blur,
 * the live reflow, the landing zone) waits for this to clear. A capture that stalls stops holding it back
 * after `CAPTURE_GATE_MAX_MS`. No dependencies, so the layout can read it without the drag modules.
 */

import { createSignal } from "solid-js";

/** The longest a capture holds the drag's drawing back. */
export const CAPTURE_GATE_MAX_MS = 400;

const [inFlight, setInFlight] = createSignal(false);
let current = 0;

/** Whether a pane's picture is being taken now. Reactive. */
export const paneCaptureInFlight = inFlight;

/** A capture starts: returns what to call when it has finished or failed. A newer capture replaces an older one. */
export function beginPaneCapture(): () => void {
    const mine = ++current;
    setInFlight(true);
    const done = () => {
        if (mine !== current) return;
        current++;
        clearTimeout(timer);
        setInFlight(false);
    };
    const timer = setTimeout(done, CAPTURE_GATE_MAX_MS);
    return done;
}

/** Test hook. */
export function resetPaneCaptureGateForTests(): void {
    current++;
    setInFlight(false);
}
