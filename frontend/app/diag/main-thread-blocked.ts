// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * How long this renderer's main thread was blocked inside a window of time, from the browser's Long Tasks entries
 * (a task over 50 ms). The tab-switch line reads it when the SetActiveTab call came back slowly, to say whether the
 * wait was the backend or this thread being busy (#47767). Its own observer, so it doesn't depend on the perf HUD
 * having started the shared one.
 */

interface LongTask {
    start: number;
    end: number;
}

// A switch window is seconds long at the worst; 64 tasks cover it many times over.
const RING_MAX = 64;
let ring: LongTask[] = [];
let observer: PerformanceObserver | null = null;
let started = false;

function addEntries(entries: PerformanceEntryList): void {
    for (const e of entries) ring.push({ start: e.startTime, end: e.startTime + e.duration });
    if (ring.length > RING_MAX) ring = ring.slice(ring.length - RING_MAX);
}

/** Start recording long tasks. Safe to call again, and where the engine has no Long Tasks API it records nothing. */
export function startBlockedTracking(): void {
    if (started) return;
    started = true;
    if (typeof PerformanceObserver === "undefined") return;
    const types = (PerformanceObserver as unknown as { supportedEntryTypes?: string[] }).supportedEntryTypes;
    if (!Array.isArray(types) || !types.includes("longtask")) return;
    try {
        observer = new PerformanceObserver((list) => addEntries(list.getEntries()));
        observer.observe({ type: "longtask" });
    } catch {
        observer = null;
    }
}

/** Milliseconds of long tasks that fall inside [start, end] (performance.now() times), or null when none is tracked. */
export function blockedMsBetween(start: number, end: number): number | null {
    if (observer == null) return null;
    // Entries reach the callback in batches at idle; take what is pending so a window that just ended is complete.
    addEntries(observer.takeRecords());
    let total = 0;
    for (const t of ring) {
        const overlap = Math.min(t.end, end) - Math.max(t.start, start);
        if (overlap > 0) total += overlap;
    }
    return Math.round(total);
}

/** The longest long task (ms) that started inside [start, end] (performance.now() times), or null when none is tracked. */
export function longestTaskStartedBetween(start: number, end: number): number | null {
    if (observer == null) return null;
    addEntries(observer.takeRecords());
    let longest = 0;
    for (const t of ring) if (t.start >= start && t.start <= end) longest = Math.max(longest, t.end - t.start);
    return Math.round(longest);
}

/** Test seam: forget everything and let the next start begin afresh. */
export function resetBlockedTracking(): void {
    observer?.disconnect();
    observer = null;
    ring = [];
    started = false;
}
