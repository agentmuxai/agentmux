// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Runs slow work that can wait when the app is not busy: after the first frame has been drawn, and then in idle time.
 * Opening the audio device (`new AudioContext()`) is such work (after-gesture.ts): it blocks the page for as long as
 * the machine's audio stack takes, and the first press or key of a session is the worst moment for that. Run here,
 * the cost lands after the window has drawn and before the person has started anything.
 */

/** Idle time is waited for at most this long; a page that never idles still runs the work. */
export const IDLE_TIMEOUT_MS = 5000;

/** Without `requestIdleCallback`, the work runs this long after the second frame. */
export const NO_IDLE_API_DELAY_MS = 250;

function nextFrame(fn: () => void): void {
    if (typeof requestAnimationFrame === "function") requestAnimationFrame(() => fn());
    else setTimeout(fn, 16);
}

/**
 * Call `run` once, after two frames have been drawn (the first one is the app's first paint) and the page is idle.
 * Returns a function that cancels it; it does nothing once `run` has been called.
 */
export function whenIdleAfterFirstPaint(run: () => void): () => void {
    let cancelled = false;
    let idle: number | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const go = () => {
        if (!cancelled) run();
    };
    nextFrame(() =>
        nextFrame(() => {
            if (cancelled) return;
            if (typeof requestIdleCallback === "function") idle = requestIdleCallback(go, { timeout: IDLE_TIMEOUT_MS });
            else timer = setTimeout(go, NO_IDLE_API_DELAY_MS);
        })
    );

    return () => {
        cancelled = true;
        if (idle !== undefined && typeof cancelIdleCallback === "function") cancelIdleCallback(idle);
        clearTimeout(timer);
    };
}
