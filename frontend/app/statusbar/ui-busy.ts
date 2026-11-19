// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "UI n%": the share of the last second this window's page spent in long tasks (the browser's Long Tasks entries,
 * which are tasks of 50 ms or more), from the observer `diag/main-thread-blocked.ts` already keeps. It answers "is
 * the page itself the slow part": the status bar's other meters are the machine's, and a page can stall with every one
 * of them cool. It is this window's page only: not an agent, not srv, not another window.
 *
 * A window in the background has nothing to show: the browser throttles its timers and its tasks, so a reading from
 * it would say "0%" for a page nobody can see. It shows no figure until it is visible again, and the first second after
 * it returns is not counted (that window's long tasks were waiting, not this second's).
 */

import { blockedMsBetween, startBlockedTracking } from "@/app/diag/main-thread-blocked";
import { createRoot, createSignal, onCleanup, onMount, type Accessor } from "solid-js";

/** Above this share the meter turns the warning colour. */
export const UI_BUSY_WARN_PCT = 50;
/** How often the figure is made. */
export const UI_BUSY_EVERY_MS = 1000;

/** The share of a window of `windowMs` that `blockedMs` of long tasks fill, whole percent within 0 to 100. */
export function busyPercent(blockedMs: number, windowMs: number): number {
    if (windowMs <= 0) return 0;
    return Math.round(Math.min(100, Math.max(0, (blockedMs / windowMs) * 100)));
}

/** The meter's colour. */
export function uiBusyColor(pct: number): string {
    return pct > UI_BUSY_WARN_PCT ? "var(--warning-text-color)" : "var(--secondary-text-color)";
}

/** What the meter reads from the world, so a test can give it a clock. */
export interface UiBusyEnv {
    now: () => number;
    visible: () => boolean;
    /** Long-task milliseconds inside [start, end], or null where the engine has no Long Tasks API. */
    blockedBetween: (start: number, end: number) => number | null;
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
            const blocked = env.blockedBetween(start, now);
            return blocked == null ? null : busyPercent(blocked, now - start);
        },
    };
}

let shared: Accessor<number | null> | null = null;

/** This window's one meter, made on first use and kept: the status bar's "UI n%" and the GPU panel's footer read the same figure. */
export function sharedUiBusy(): Accessor<number | null> {
    shared ??= createRoot(() => createUiBusy());
    return shared;
}

/** The meter for this window's page, as a signal: `null` while hidden, before the first second, or without the API. */
export function createUiBusy(): Accessor<number | null> {
    const [busy, setBusy] = createSignal<number | null>(null);
    onMount(() => {
        startBlockedTracking();
        const meter = createUiBusyMeter({
            now: () => performance.now(),
            visible: () => document.visibilityState === "visible",
            blockedBetween: blockedMsBetween,
        });
        meter.tick();
        const timer = setInterval(() => setBusy(meter.tick()), UI_BUSY_EVERY_MS);
        onCleanup(() => clearInterval(timer));
    });
    return busy;
}
