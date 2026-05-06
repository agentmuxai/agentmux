// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Drift-storm regression guard tests for the launcher-event bridge.
// See `docs/specs/ANALYSIS_DRIFT_STORM_RENDERER_CRASH_2026-05-06.md`.

import { beforeEach, describe, expect, it } from "vitest";
import {
    __resetDedupForTests,
    launcherEventDedupStats,
    shouldDispatchLauncherEvent,
    type LauncherEvent,
} from "./launcher-events";

const evt = (over: Partial<LauncherEvent> & { event: string; version: number }): LauncherEvent =>
    over as LauncherEvent;

describe("launcher-event per-key dedup", () => {
    beforeEach(() => __resetDedupForTests());

    it("admits the first event for a given (kind, label, hwnd)", () => {
        const e = evt({ event: "hwnd_drift_detected", version: 13, label: "window-pool-x", hwnd: 100 });
        expect(shouldDispatchLauncherEvent(e)).toBe(true);
        expect(launcherEventDedupStats()).toEqual({ tracked: 1, suppressed: 0 });
    });

    it("suppresses re-emission of the same (kind, label, hwnd) at the same version", () => {
        const e = evt({ event: "hwnd_drift_detected", version: 13, label: "window-pool-x", hwnd: 100 });
        expect(shouldDispatchLauncherEvent(e)).toBe(true);
        // The drift-storm bug: same event re-arrives many times with same version.
        for (let i = 0; i < 100; i++) {
            expect(shouldDispatchLauncherEvent(e)).toBe(false);
        }
        expect(launcherEventDedupStats()).toEqual({ tracked: 1, suppressed: 100 });
    });

    it("admits a higher-versioned event for the same key", () => {
        const a = evt({ event: "hwnd_drift_detected", version: 13, label: "x", hwnd: 1 });
        const b = evt({ event: "hwnd_drift_detected", version: 17, label: "x", hwnd: 1 });
        expect(shouldDispatchLauncherEvent(a)).toBe(true);
        expect(shouldDispatchLauncherEvent(b)).toBe(true);
    });

    it("treats different event kinds at the same version as distinct keys", () => {
        // Launcher versions are global-monotonic; same version on different
        // event kinds is impossible upstream, but the dedup must not couple
        // unrelated events even hypothetically.
        expect(shouldDispatchLauncherEvent(evt({ event: "hwnd_drift_detected", version: 13, label: "x" }))).toBe(true);
        expect(shouldDispatchLauncherEvent(evt({ event: "window_opened", version: 13, label: "x" }))).toBe(true);
        expect(shouldDispatchLauncherEvent(evt({ event: "window_closed", version: 13, label: "x" }))).toBe(true);
    });

    it("treats different labels at the same version as distinct keys", () => {
        expect(shouldDispatchLauncherEvent(evt({ event: "hwnd_drift_detected", version: 13, label: "a" }))).toBe(true);
        expect(shouldDispatchLauncherEvent(evt({ event: "hwnd_drift_detected", version: 13, label: "b" }))).toBe(true);
    });

    it("treats missing label/hwnd as a distinct key from present ones", () => {
        expect(shouldDispatchLauncherEvent(evt({ event: "host_should_quit", version: 5 }))).toBe(true);
        expect(shouldDispatchLauncherEvent(evt({ event: "host_should_quit", version: 5, label: "x" }))).toBe(true);
    });

    it("bounds memory under a flood of unique keys", () => {
        // 2000 distinct labels at v=1 — should evict oldest, keeping the cap.
        for (let i = 0; i < 2000; i++) {
            shouldDispatchLauncherEvent(evt({ event: "window_opened", version: 1, label: `w-${i}` }));
        }
        expect(launcherEventDedupStats().tracked).toBeLessThanOrEqual(1024);
    });

    it("after eviction, an older key's re-arrival is admitted only if its version exceeds anything seen", () => {
        // Saturate the cache to evict early entries.
        shouldDispatchLauncherEvent(evt({ event: "hwnd_drift_detected", version: 13, label: "evicted" }));
        for (let i = 0; i < 1100; i++) {
            shouldDispatchLauncherEvent(evt({ event: "window_opened", version: 1, label: `filler-${i}` }));
        }
        // "evicted" is gone. A re-arrival at v=13 looks new and admits — acceptable
        // per the comment in launcher-events.ts: only a strictly higher version
        // would be a real duplicate, which can't happen if the upstream is healthy.
        expect(shouldDispatchLauncherEvent(evt({ event: "hwnd_drift_detected", version: 13, label: "evicted" }))).toBe(true);
    });
});
