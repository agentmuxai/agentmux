// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * How long this renderer spent in slow frames inside a window of time, from the browser's Long Animation Frames
 * entries: a frame that took 50 ms or more from its first task to its paint, which is about three frames missed at
 * 60 Hz. A frame is what a person sees, and it is made of several tasks and a rendering step. Long tasks alone
 * (`main-thread-blocked.ts`) miss a frame filled by many short tasks: a page drawing four or five frames a second read
 * as under half busy by long tasks (docs/reports/REPORT_UI_CHOP_ON_FLEET_BROADCAST_AND_UI_BUSY_METER_2026_11_19.md).
 *
 * A frame counts whole in the window it ended in: its entry only exists once it has ended, so a frame that began in
 * one window and ended in the next would otherwise lose the part that fell in the window already read.
 */

interface SlowFrame {
    end: number;
    duration: number;
}

// Slow frames are 50 ms or more each, so a late reading that covers ten seconds holds 200 at the very most.
const RING_MAX = 256;
let ring: SlowFrame[] = [];
let observer: PerformanceObserver | null = null;
let started = false;

function addEntries(entries: PerformanceEntryList): void {
    for (const e of entries) ring.push({ end: e.startTime + e.duration, duration: e.duration });
    if (ring.length > RING_MAX) ring = ring.slice(ring.length - RING_MAX);
}

/** Start recording slow frames. Safe to call again; where the engine has no Long Animation Frames it records nothing. */
export function startSlowFrameTracking(): void {
    if (started) return;
    started = true;
    if (typeof PerformanceObserver === "undefined") return;
    const types = (PerformanceObserver as unknown as { supportedEntryTypes?: string[] }).supportedEntryTypes;
    if (!Array.isArray(types) || !types.includes("long-animation-frame")) return;
    try {
        observer = new PerformanceObserver((list) => addEntries(list.getEntries()));
        observer.observe({ type: "long-animation-frame" });
    } catch {
        observer = null;
    }
}

/**
 * Milliseconds of slow frames that ended after `start` and at or before `end` (performance.now() times), each counted
 * whole; null when none is tracked. The sum can exceed the window when a frame that began before it ended inside it.
 */
export function slowFrameMsEndedBetween(start: number, end: number): number | null {
    if (observer == null) return null;
    // Entries reach the callback in batches at idle; take what is pending so a window that just ended is complete.
    addEntries(observer.takeRecords());
    let total = 0;
    for (const f of ring) {
        if (f.end > start && f.end <= end) total += f.duration;
    }
    return Math.round(total);
}

/** Test seam: forget everything and let the next start begin afresh. */
export function resetSlowFrameTracking(): void {
    observer?.disconnect();
    observer = null;
    ring = [];
    started = false;
}
