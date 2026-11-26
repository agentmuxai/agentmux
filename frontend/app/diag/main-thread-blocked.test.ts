// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { blockedMsBetween, longestTaskStartedBetween, resetBlockedTracking, startBlockedTracking } from "./main-thread-blocked";

type Entry = { startTime: number; duration: number };

let pending: Entry[] = [];
let delivered: ((list: { getEntries: () => Entry[] }) => void) | null = null;

class FakeObserver {
    static supportedEntryTypes = ["longtask"];
    constructor(cb: (list: { getEntries: () => Entry[] }) => void) {
        delivered = cb;
    }
    observe() {}
    disconnect() {}
    takeRecords() {
        const out = pending;
        pending = [];
        return out;
    }
}

describe("main-thread-blocked", () => {
    beforeEach(() => {
        pending = [];
        delivered = null;
        resetBlockedTracking();
        vi.stubGlobal("PerformanceObserver", FakeObserver);
    });
    afterEach(() => {
        resetBlockedTracking();
        vi.unstubAllGlobals();
    });

    it("reports null while nothing is tracked", () => {
        expect(blockedMsBetween(0, 1000)).toBeNull();
    });

    it("reports null where the engine has no long task entries", () => {
        class NoLongTask extends FakeObserver {
            static supportedEntryTypes = ["event"];
        }
        vi.stubGlobal("PerformanceObserver", NoLongTask);
        startBlockedTracking();
        expect(blockedMsBetween(0, 1000)).toBeNull();
    });

    it("sums the long-task time that falls inside the window, clipped at its edges", () => {
        startBlockedTracking();
        delivered?.({
            getEntries: () => [
                { startTime: 100, duration: 80 }, // wholly inside
                { startTime: 950, duration: 100 }, // runs past the end: 50 inside
                { startTime: 10, duration: 60 }, // before the window
            ],
        });
        expect(blockedMsBetween(90, 1000)).toBe(130);
    });

    it("names the longest task that started inside the window, whole, and null where nothing is tracked", () => {
        expect(longestTaskStartedBetween(0, 1000)).toBeNull();
        startBlockedTracking();
        delivered?.({
            getEntries: () => [
                { startTime: 100, duration: 80 },
                { startTime: 900, duration: 200 }, // starts inside, runs past the end: counts whole
                { startTime: 10, duration: 500 }, // starts before the window
            ],
        });
        expect(longestTaskStartedBetween(90, 1000)).toBe(200);
        expect(longestTaskStartedBetween(2000, 3000)).toBe(0);
    });

    it("takes entries not yet delivered to the callback", () => {
        startBlockedTracking();
        pending.push({ startTime: 200, duration: 300 });
        expect(blockedMsBetween(0, 1000)).toBe(300);
        expect(blockedMsBetween(0, 1000)).toBe(300);
    });

    it("keeps no more than the latest 64 tasks", () => {
        startBlockedTracking();
        const entries: Entry[] = [];
        for (let i = 0; i < 100; i++) entries.push({ startTime: i * 100, duration: 10 });
        delivered?.({ getEntries: () => entries });
        // The first 36 are gone: only tasks 36..99 (64 of them, 10 ms each) remain.
        expect(blockedMsBetween(0, 100000)).toBe(640);
    });
});
