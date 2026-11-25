// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * When a press on a pane's header is worth a picture of the pane (tearoff-snapshot.ts). The picture is a screenshot of
 * the whole window, which takes the host 0.1 to 1.3 s and holds up the page's frames, so a press that is only a click
 * must not pay for it: it was taken at every pointer down. A press that may become a drag is armed here instead, and the
 * picture is taken when the pointer has moved a few pixels (the drag is starting, before the drag draws anything: the
 * drag's own drawing waits for the capture, pane-capture-gate.ts), when the library's drag starts, or when the pointer
 * has been held down for a moment. A release before any of those takes no picture.
 * docs/reports/REPORT_TEAR_OFF_LATENCY_AUDIT_2026_11_24.md §2.1.
 */

/** How far the pointer moves from its press before the press is read as the start of a drag (CSS px). */
export const PRESS_DRAG_PX = 3;
/** How long a press held without moving is read as a drag about to begin. */
export const PRESS_HOLD_MS = 150;

let stopArmed: (() => void) | undefined;

/** Forget the press being watched, if any: no picture will be taken for it. */
export function disarmPressCapture(): void {
    stopArmed?.();
}

/**
 * Watch the press at `point` (viewport px) and call `capture` once, when it becomes a drag or a hold. Never when it
 * ends first. One press is watched at a time: arming another forgets the earlier one.
 */
export function armPressCapture(point: { x: number; y: number }, capture: () => void): void {
    disarmPressCapture();
    const listeners: Array<[string, (e: Event) => void]> = [];
    let timer: ReturnType<typeof setTimeout> | undefined;
    const stop = () => {
        clearTimeout(timer);
        for (const [type, fn] of listeners) document.removeEventListener(type, fn, true);
        if (stopArmed === stop) stopArmed = undefined;
    };
    const trigger = () => {
        stop();
        capture();
    };
    const moved = (e: Event) => {
        const m = e as MouseEvent;
        if (Math.hypot(m.clientX - point.x, m.clientY - point.y) >= PRESS_DRAG_PX) trigger();
    };
    // A native drag cancels the pointer once it has begun, so a cancel is a drag that has begun.
    listeners.push(["pointermove", moved], ["dragstart", trigger], ["pointercancel", trigger], ["pointerup", stop]);
    for (const [type, fn] of listeners) document.addEventListener(type, fn, true);
    timer = setTimeout(trigger, PRESS_HOLD_MS);
    stopArmed = stop;
}
