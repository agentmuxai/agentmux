// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "UI n%": the share of the last second this window's page spent in slow frames, a slow frame being one that took
 * 50 ms or more from its first task to its paint (`diag/slow-frames.ts`, the browser's Long Animation Frames). It
 * answers "is the page itself the slow part": the status bar's other meters are the machine's, and a page can stall
 * with every one of them cool. It is this window's page only: not an agent, not srv, not another window.
 *
 * It counts frames and not long tasks because a frame is what a person sees. A frame filled by several tasks of 10 to
 * 40 ms and a long rendering step holds no long task at all: counted by long tasks, a page drawing four or five frames
 * a second read 44% (docs/reports/REPORT_UI_CHOP_ON_FLEET_BROADCAST_AND_UI_BUSY_METER_2026_11_19.md). Where the
 * engine has no Long Animation Frames the figure falls back to long tasks (`diag/main-thread-blocked.ts`).
 *
 * A window in the background has nothing to show: the browser throttles its timers and its tasks, so a reading from
 * it would say "0%" for a page nobody can see. It shows no figure until it is visible again, and the first second after
 * it returns is not counted (that window's long tasks were waiting, not this second's).
 */

import { blockedMsBetween, startBlockedTracking } from "@/app/diag/main-thread-blocked";
import { slowFrameMsEndedBetween, startSlowFrameTracking } from "@/app/diag/slow-frames";
import { createRoot, createSignal, onCleanup, onMount, type Accessor } from "solid-js";

/** Above this share the meter turns the warning colour. */
export const UI_BUSY_WARN_PCT = 50;
/** How often the figure is made. */
export const UI_BUSY_EVERY_MS = 1000;

/** The share of a window of `windowMs` that `busyMs` of slow frames fill, whole percent within 0 to 100. */
export function busyPercent(busyMs: number, windowMs: number): number {
    if (windowMs <= 0) return 0;
    return Math.round(Math.min(100, Math.max(0, (busyMs / windowMs) * 100)));
}

/** The meter's colour. */
export function uiBusyColor(pct: number): string {
    return pct > UI_BUSY_WARN_PCT ? "var(--warning-text-color)" : "var(--secondary-text-color)";
}

/** What the meter reads from the world, so a test can give it a clock. */
export interface UiBusyEnv {
    now: () => number;
    visible: () => boolean;
    /** Milliseconds of slow frames in the window from `start` to `end`, or null where the engine can't say. */
    busyBetween: (start: number, end: number) => number | null;
}

/** The meter's rule: one reading per tick, `null` while there is nothing to say. */
export function createUiBusyMeter(env: UiBusyEnv): { tick: () => number | null } {
    let from: number | null = null;
    return {
        tick() {
            const now = env.now();
            if (!env.visible()) {
                from = null;
                return null;
            }
            const start = from;
            from = now;
            if (start == null) return null;
            const busy = env.busyBetween(start, now);
            return busy == null ? null : busyPercent(busy, now - start);
        },
    };
}

let shared: Accessor<number | null> | null = null;

/** This window's one meter, made on first use and kept: the status bar's "UI n%" and the GPU panel's footer read the same figure. */
export function sharedUiBusy(): Accessor<number | null> {
    shared ??= createRoot(() => createUiBusy());
    return shared;
}

/** Slow-frame time in a window; long-task time where the engine has no Long Animation Frames; null with neither. */
export function pageBusyBetween(start: number, end: number): number | null {
    return slowFrameMsEndedBetween(start, end) ?? blockedMsBetween(start, end);
}

/** The meter for this window's page, as a signal: `null` while hidden, before the first second, or without the API. */
export function createUiBusy(): Accessor<number | null> {
    const [busy, setBusy] = createSignal<number | null>(null);
    onMount(() => {
        startSlowFrameTracking();
        startBlockedTracking();
        const meter = createUiBusyMeter({
            now: () => performance.now(),
            visible: () => document.visibilityState === "visible",
            busyBetween: pageBusyBetween,
        });
        meter.tick();
        const timer = setInterval(() => setBusy(meter.tick()), UI_BUSY_EVERY_MS);
        onCleanup(() => clearInterval(timer));
    });
    return busy;
}
