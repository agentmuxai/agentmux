// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Runs work that is slow and can wait until the gesture that unlocked it is over. Opening the audio device
 * (`new AudioContext()`) blocks the page for as long as the machine's audio stack takes: about 0.1 s to 1 s in
 * the installed builds' logs, and 16 s once, on a debug build, in the middle of a press that was starting a pane
 * drag. Run inside the press's own handler it freezes whatever the press began: the drag's guides, its preview,
 * its drop. Run after the press is over, and after any drag it turned into, the same cost lands at a moment the
 * person is not in the middle of anything.
 */

/** How long after the release (or the key) before the work runs, so a click that turns into another gesture is not caught. */
export const SETTLE_MS = 500;

/** A press whose end is never seen (the OS lost it) does not hold the work back longer than this. */
export const MAX_WAIT_MS = 30_000;

/**
 * Call `run` once, after the gesture `e` is over: a key straight away (after the settle time), a press when the
 * pointer is released and no drag it started is still going.
 */
export function afterGesture(e: Pick<Event, "type">, run: () => void): void {
    let pressed = e.type === "pointerdown";
    let dragging = false;
    let settle: ReturnType<typeof setTimeout> | undefined;
    let cap: ReturnType<typeof setTimeout> | undefined;

    const finish = () => {
        clearTimeout(settle);
        clearTimeout(cap);
        for (const [type, fn] of listeners) document.removeEventListener(type, fn, true);
        run();
    };
    const check = () => {
        // Still held or dragging: the next release or drag end arms the settle again.
        if (!pressed && !dragging) finish();
    };
    const arm = () => {
        clearTimeout(settle);
        settle = setTimeout(check, SETTLE_MS);
    };

    const listeners: Array<[string, () => void]> = [
        ["dragstart", () => void (dragging = true)],
        [
            "dragend",
            () => {
                dragging = false;
                pressed = false;
                arm();
            },
        ],
        [
            "pointerup",
            () => {
                pressed = false;
                arm();
            },
        ],
        // A press that turns into a drag is cancelled as the drag starts: `dragstart` has already said so.
        [
            "pointercancel",
            () => {
                pressed = false;
                arm();
            },
        ],
    ];
    for (const [type, fn] of listeners) document.addEventListener(type, fn, true);
    cap = setTimeout(finish, MAX_WAIT_MS);
    if (!pressed) arm();
}
