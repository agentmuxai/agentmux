// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { resetSlowFrameTracking, slowFrameMsEndedBetween, startSlowFrameTracking } from "./slow-frames";

type Entry = { startTime: number; duration: number };

let pending: Entry[] = [];
let delivered: ((list: { getEntries: () => Entry[] }) => void) | null = null;
let observed: unknown = null;

class FakeObserver {
    static supportedEntryTypes = ["longtask", "long-animation-frame"];
    constructor(cb: (list: { getEntries: () => Entry[] }) => void) {
        delivered = cb;
    }
    observe(options: unknown) {
        observed = options;
    }
    disconnect() {}
    takeRecords() {
        const out = pending;
        pending = [];
        return out;
    }
}

describe("slow-frames", () => {
    beforeEach(() => {
        pending = [];
        delivered = null;
        observed = null;
        resetSlowFrameTracking();
        vi.stubGlobal("PerformanceObserver", FakeObserver);
    });
    afterEach(() => {
        resetSlowFrameTracking();
        vi.unstubAllGlobals();
    });

    it("reports null while nothing is tracked", () => {
        expect(slowFrameMsEndedBetween(0, 1000)).toBeNull();
    });

    it("reports null where the engine has no Long Animation Frames, so the caller can fall back", () => {
        class LongTasksOnly extends FakeObserver {
            static supportedEntryTypes = ["longtask"];
        }
        vi.stubGlobal("PerformanceObserver", LongTasksOnly);
        startSlowFrameTracking();
        expect(slowFrameMsEndedBetween(0, 1000)).toBeNull();
    });

    it("observes long animation frames", () => {
        startSlowFrameTracking();
        expect(observed).toEqual({ type: "long-animation-frame" });
    });

    it("counts a frame whole in the window it ended in", () => {
        startSlowFrameTracking();
        delivered?.({
            getEntries: () => [
                { startTime: 100, duration: 80 }, // ends at 180: inside
                { startTime: 900, duration: 250 }, // ends at 1150: the next window's
                { startTime: -200, duration: 260 }, // began before the window, ended at 60 inside it: whole
                { startTime: -300, duration: 300 }, // ended exactly at the window's start: the one before's
            ],
        });
        expect(slowFrameMsEndedBetween(0, 1000)).toBe(340);
        expect(slowFrameMsEndedBetween(1000, 2000)).toBe(250);
    });

    it("takes entries not yet delivered to the callback, once", () => {
        startSlowFrameTracking();
        pending.push({ startTime: 200, duration: 300 });
        expect(slowFrameMsEndedBetween(0, 1000)).toBe(300);
        expect(slowFrameMsEndedBetween(0, 1000)).toBe(300);
    });

    it("keeps no more than the latest 256 frames", () => {
        startSlowFrameTracking();
        const entries: Entry[] = [];
        for (let i = 0; i < 300; i++) entries.push({ startTime: i * 100, duration: 50 });
        delivered?.({ getEntries: () => entries });
        // The first 44 are gone: 256 frames of 50 ms remain.
        expect(slowFrameMsEndedBetween(0, 1_000_000)).toBe(12800);
    });
});
