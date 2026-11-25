// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether the picture of a pane about to be dragged is still being taken (tearoff-snapshot.ts). The picture is
 * a capture of the window as it is drawn when the capture runs, which is some 50 to 250 ms after the press,
 * and a quick drag starts inside that time. Whatever the drag draws over the pane by then is in the picture
 * the user then holds: on 2026-11-10 the operator held a blurred pane, because the dragged tile's blur
 * (tilelayout.scss `.dragging`, a blur since removed) was on screen before the capture ran. So the drag's own
 * drawing (the tear-off place, the live reflow, the landing zone) waits for this to clear. A capture that stalls stops holding it back
 * after `CAPTURE_GATE_MAX_MS`. No dependencies, so the layout can read it without the drag modules.
 */

import { Logger } from "@/util/logger";
import { createSignal } from "solid-js";

/** The longest a capture holds the drag's drawing back. */
export const CAPTURE_GATE_MAX_MS = 400;

const [inFlight, setInFlight] = createSignal(false);
let current = 0;

/** How long a picture may have been ready before a drag began for the drag to be logged against it. */
const READY_WINDOW_MS = 10_000;
/** When a drag began while a picture was being taken: the drag's drawing is held from then until the picture is done. */
let dragBeganAt: number | undefined;
/** When the last picture was done. */
let lastDoneAt = 0;

/**
 * The native drag began (the drag library's own `dragstart`). Logs, in the `[tearoff-perf]` family, how long the
 * drag's drawing is held for the picture: now if the picture is still being taken (the line comes when it is done),
 * none if it was ready before the drag began. A drag the operator makes answers whether the first drawn frame waits.
 */
function onDragStart(): void {
    const now = Date.now();
    if (inFlight()) {
        dragBeganAt = now;
        return;
    }
    if (lastDoneAt && now - lastDoneAt < READY_WINDOW_MS) {
        Logger.info("dnd:cross", "[tearoff-perf] the drag's drawing was not held: the picture was ready before the drag began", {
            heldMs: 0,
            readyBeforeMs: now - lastDoneAt,
        });
    }
}
if (typeof document !== "undefined") document.addEventListener("dragstart", onDragStart, true);

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
        lastDoneAt = Date.now();
        if (dragBeganAt !== undefined) {
            const heldMs = lastDoneAt - dragBeganAt;
            dragBeganAt = undefined;
            Logger.info("dnd:cross", "[tearoff-perf] the drag's drawing was held for the picture", { heldMs, timedOut: heldMs >= CAPTURE_GATE_MAX_MS });
        }
    };
    const timer = setTimeout(done, CAPTURE_GATE_MAX_MS);
    return done;
}

/** Test hook. */
export function resetPaneCaptureGateForTests(): void {
    current++;
    setInFlight(false);
    dragBeganAt = undefined;
    lastDoneAt = 0;
}
