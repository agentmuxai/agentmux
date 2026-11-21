// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Whether the page was frozen during the drag that has just ended. A page that cannot run for seconds draws no
 * drop guides, handles no `dragover`, and shows the person nothing to aim at, so what they release over is not
 * a choice they made: a pane released over its own place, or over nothing, must not be torn off into a window
 * they never asked for (the 2026-11-19 report: a press that froze the window for about 16 s, ending as a tear-off).
 *
 * A heartbeat runs from a press until the drag ends. A page that could not run has a heartbeat that is old by
 * the time anything reads it, whichever of the two runs first once the freeze is over, so the verdict does not
 * depend on the order the queued events and the timer come back in. The verdict is kept until the next press.
 */

/** A freeze this long during a press or drag means the person had nothing to aim at. */
export const STALL_MS = 2000;

/** How often the heartbeat runs. */
const TICK_MS = 200;

/** A press that never ends in a `dragend` or `pointerup` (the OS lost it) stops being watched after this. */
const WATCH_MAX_MS = 120_000;

let tickTimer: ReturnType<typeof setInterval> | undefined;
let capTimer: ReturnType<typeof setTimeout> | undefined;
let lastTick = Number.NEGATIVE_INFINITY;
let stalled = false;

function noteAge(): void {
    if (performance.now() - lastTick >= STALL_MS) stalled = true;
}

/** A press: start watching. Forgets the last drag's verdict. */
export function startDragStallWatch(): void {
    stopTimers();
    stalled = false;
    lastTick = performance.now();
    tickTimer = setInterval(() => {
        noteAge();
        lastTick = performance.now();
    }, TICK_MS);
    capTimer = setTimeout(stopDragStallWatch, WATCH_MAX_MS);
}

function stopTimers(): void {
    if (tickTimer !== undefined) clearInterval(tickTimer);
    if (capTimer !== undefined) clearTimeout(capTimer);
    tickTimer = undefined;
    capTimer = undefined;
}

/** The press or drag is over: settle the verdict and stop the heartbeat. */
export function stopDragStallWatch(): void {
    if (tickTimer !== undefined) noteAge();
    stopTimers();
}

/** Whether the page was frozen for {@link STALL_MS} or more during the last press or drag. */
export function dragWasStalled(): boolean {
    if (tickTimer !== undefined) noteAge();
    return stalled;
}

/** Forget everything, so tests start clean. */
export function resetDragStallForTest(): void {
    stopTimers();
    stalled = false;
    lastTick = Number.NEGATIVE_INFINITY;
}

if (typeof document !== "undefined") {
    // Capture, on the document: before anything on the page can stop the event or block it.
    document.addEventListener("pointerdown", (e) => e.isTrusted && startDragStallWatch(), true);
    document.addEventListener("pointerup", stopDragStallWatch, true);
    document.addEventListener("dragend", stopDragStallWatch, true);
}
