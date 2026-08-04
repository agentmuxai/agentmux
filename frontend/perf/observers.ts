// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * `PerformanceObserver` subscriptions for Long Tasks (A2) and INP /
 * event timing (A3). Fed into the global perf store so the HUD can
 * read aggregated stats without each consumer subscribing
 * individually.
 *
 * Frame budget at 60 Hz is 16.67 ms. Long Tasks API surfaces any
 * single task >50 ms — by the time we hit that we've already missed
 * 3 frames, so `longtask` events are always interesting.
 *
 * INP observer uses `event` (not the older `first-input`) — it covers
 * every interaction, not just the first.
 */

import { perfStore } from "./store";

const PERF_LOG_PREFIX = "[perf]";

/** Browser-feature gates. Long Tasks API isn't in every embed; INP
 *  observer takes the same `event` entry type that's been stable in
 *  Chromium since 109. */
function supportsObserver(type: string): boolean {
    if (typeof PerformanceObserver === "undefined") return false;
    const supported = (PerformanceObserver as any).supportedEntryTypes as string[] | undefined;
    return Array.isArray(supported) && supported.includes(type);
}

function startLongTaskObserver(): (() => void) | null {
    if (!supportsObserver("longtask")) {
        console.info(`${PERF_LOG_PREFIX} longtask observer unavailable in this runtime`);
        return null;
    }
    const observer = new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) {
            // Anything ≥50 ms is a long task by definition. Surface
            // immediately — don't let the HUD's 1 Hz refresh delay
            // diagnostic info during active investigation.
            console.warn(
                `${PERF_LOG_PREFIX} long-task ${entry.duration.toFixed(1)}ms ` +
                    `name=${entry.name} startTime=${entry.startTime.toFixed(1)}`,
            );
            perfStore.recordLongTask(entry.duration);
        }
    });
    try {
        observer.observe({ type: "longtask", buffered: true });
    } catch (e) {
        console.warn(`${PERF_LOG_PREFIX} longtask observer failed to attach:`, e);
        return null;
    }
    return () => observer.disconnect();
}

function startEventObserver(): (() => void) | null {
    // The `event` entry type covers every event-driven interaction
    // (click, keydown, pointerdown, etc.) and reports the
    // interactionId — so we can group multi-event interactions (a
    // pointerdown + click that share an interactionId).
    //
    // The `durationThreshold` filter is supported on Chromium ≥106;
    // 16 ms = one frame. Below that we don't care.
    if (!supportsObserver("event")) {
        console.info(`${PERF_LOG_PREFIX} event observer unavailable in this runtime`);
        return null;
    }
    const observer = new PerformanceObserver((list) => {
        for (const entry of list.getEntries() as PerformanceEventTiming[]) {
            // Skip events with no interactionId — those are paint-only
            // events (scrollstart, etc.) that don't reflect user input.
            const interactionId =
                (entry as { interactionId?: number }).interactionId ?? 0;
            if (interactionId === 0) continue;
            perfStore.recordInteraction(entry.name, entry.duration);
        }
    });
    try {
        observer.observe({
            type: "event",
            buffered: true,
            // Don't fire below 16 ms — frame budget. Saves a lot of
            // observer churn on fast pointer/keyboard events.
            durationThreshold: 16,
        } as any);
    } catch (e) {
        console.warn(`${PERF_LOG_PREFIX} event observer failed to attach:`, e);
        return null;
    }
    return () => observer.disconnect();
}

/**
 * Attach all Phase-0 observers. Call once at startup. Returns a cleanup
 * function that disconnects them — primarily for tests; production never
 * tears these down.
 */
export function startAllObservers(): () => void {
    const stops = [startLongTaskObserver(), startEventObserver()].filter(
        (f): f is () => void => f != null,
    );
    return () => {
        for (const s of stops) s();
    };
}
