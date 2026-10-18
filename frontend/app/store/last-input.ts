// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * When the user last did something in this window: a key, a pointer press, a
 * wheel turn. The renderer recycle governor asks the page before reloading it,
 * and the page declines if the user was here recently
 * (docs/specs/SPEC_RENDERER_RECYCLE_WHEN_IDLE_2026_10_13.md §4.3).
 */

let lastInputAt = Date.now();
let installed = false;

function note(): void {
    lastInputAt = Date.now();
}

/** Start listening (once). Counting starts at the page's load. */
export function installLastInput(): void {
    if (installed || typeof window === "undefined") return;
    installed = true;
    for (const type of ["keydown", "pointerdown", "wheel"] as const) {
        window.addEventListener(type, note, { capture: true, passive: true });
    }
}

/** Milliseconds since the last input in this window, or since the page loaded. */
export function msSinceLastInput(): number {
    return Date.now() - lastInputAt;
}

/** Test-only: pretend the last input was `msAgo` milliseconds ago. */
export function setLastInputForTests(msAgo: number): void {
    lastInputAt = Date.now() - msAgo;
}
