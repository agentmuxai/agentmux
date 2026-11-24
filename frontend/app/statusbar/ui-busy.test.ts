// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it, vi } from "vitest";
import { resetBlockedTracking, startBlockedTracking } from "@/app/diag/main-thread-blocked";
import { resetSlowFrameTracking, startSlowFrameTracking } from "@/app/diag/slow-frames";
import { busyPercent, createUiBusyMeter, nextUiBusyAlert, pageBusyBetween, UI_BUSY_CALM_TICKS, UI_BUSY_WARN_PCT, uiBusyColor } from "./ui-busy";

describe("the UI busy share", () => {
    it("is the slow-frame time over the window, whole and within 0 to 100", () => {
        expect(busyPercent(120, 1000)).toBe(12);
        expect(busyPercent(1500, 1000)).toBe(100);
        expect(busyPercent(-5, 1000)).toBe(0);
        expect(busyPercent(10, 0)).toBe(0);
    });

    it("turns the warning colour above 50% and not at it", () => {
        expect(UI_BUSY_WARN_PCT).toBe(50);
        expect(uiBusyColor(50)).toBe("var(--secondary-text-color)");
        expect(uiBusyColor(51)).toBe("var(--warning-text-color)");
    });
});

describe("the busy alert behind the Graphics item's red", () => {
    const off = { on: false, calm: 0 };
    const run = (readings: Array<number | null>) => readings.reduce((s, r) => nextUiBusyAlert(s, r), off);

    it("comes on at once above the warn share, never at it", () => {
        expect(run([50]).on).toBe(false);
        expect(run([51]).on).toBe(true);
        expect(run([5, 5, 99]).on).toBe(true);
    });

    it("lets go only after UI_BUSY_CALM_TICKS calm readings in a row", () => {
        expect(UI_BUSY_CALM_TICKS).toBe(2);
        expect(run([80, 10]).on).toBe(true);
        expect(run([80, 10, 10]).on).toBe(false);
        // A busy reading between two calm ones starts the count again.
        expect(run([80, 10, 70, 10]).on).toBe(true);
        expect(run([80, 10, 70, 10, 10]).on).toBe(false);
    });

    it("is off while the window has no reading (in the background), and starts clean after it", () => {
        expect(run([80, null]).on).toBe(false);
        expect(run([80, null, 10, 10]).on).toBe(false);
    });
});

/** A clock and a page that can be hidden, with the slow-frame time the page has had in each window. */
function world(busy: (start: number, end: number) => number | null = () => 0) {
    const w = { t: 0, visible: true };
    const meter = createUiBusyMeter({ now: () => w.t, visible: () => w.visible, busyBetween: busy });
    return { w, meter };
}

describe("the UI busy meter", () => {
    it("says nothing on its first tick: there is no second to measure yet", () => {
        const { meter } = world();
        expect(meter.tick()).toBeNull();
    });

    it("reads each second's slow frames as a share of that second", () => {
        const { w, meter } = world((start, end) => (end - start) * 0.25);
        meter.tick();
        w.t = 1000;
        expect(meter.tick()).toBe(25);
        w.t = 2000;
        expect(meter.tick()).toBe(25);
    });

    it("measures a late tick against the time that really passed", () => {
        const { w, meter } = world(() => 1500);
        meter.tick();
        w.t = 3000;
        expect(meter.tick()).toBe(50);
    });

    it("shows nothing while the window is in the background, and not the first second back", () => {
        const { w, meter } = world(() => 900);
        meter.tick();
        w.t = 1000;
        expect(meter.tick()).toBe(90);
        w.visible = false;
        w.t = 2000;
        expect(meter.tick()).toBeNull();
        w.t = 3000;
        expect(meter.tick()).toBeNull();
        w.visible = true;
        w.t = 4000;
        expect(meter.tick()).toBeNull();
        w.t = 5000;
        expect(meter.tick()).toBe(90);
    });

    it("shows nothing where the engine can say neither slow frames nor long tasks", () => {
        const { w, meter } = world(() => null);
        meter.tick();
        w.t = 1000;
        expect(meter.tick()).toBeNull();
    });
});

type Entry = { startTime: number; duration: number };

/** A page whose engine reports the given entry types, with the frames and long tasks it has had. */
function page(types: string[], frames: Entry[], tasks: Entry[]) {
    class FakeObserver {
        static supportedEntryTypes = types;
        private type = "";
        observe(options: { type: string }) {
            this.type = options.type;
        }
        disconnect() {}
        takeRecords() {
            const out = this.type === "long-animation-frame" ? frames : tasks;
            if (this.type === "long-animation-frame") frames = [];
            else tasks = [];
            return out;
        }
    }
    vi.stubGlobal("PerformanceObserver", FakeObserver);
    startSlowFrameTracking();
    startBlockedTracking();
    const w = { t: 0 };
    const meter = createUiBusyMeter({ now: () => w.t, visible: () => true, busyBetween: pageBusyBetween });
    meter.tick();
    return { w, meter };
}

/** Frames one after another from `at`, `gap` ms apart, of the given lengths. */
function backToBack(durations: number[], at = 0, gap = 20): Entry[] {
    let t = at;
    return durations.map((duration) => {
        const entry = { startTime: t, duration };
        t += duration + gap;
        return entry;
    });
}

describe("the UI busy meter on recorded frames", () => {
    afterEach(() => {
        resetSlowFrameTracking();
        resetBlockedTracking();
        vi.unstubAllGlobals();
    });

    // The frames one window's page logged while 16 agents started a turn within four seconds (the report's burst):
    // 248, 245, 165 and 188 ms, one after another. The only tasks of 50 ms or more in them were 143 and 150 ms, so the
    // figure read under half while the page drew about four frames a second.
    const BURST_FRAMES = backToBack([248, 245, 165, 188]);
    const BURST_LONG_TASKS: Entry[] = [
        { startTime: 40, duration: 143 },
        { startTime: 300, duration: 150 },
    ];

    it("reads a page drawing four frames a second as busy, where long tasks alone read under a third", () => {
        const withFrames = page(["longtask", "long-animation-frame"], [...BURST_FRAMES], [...BURST_LONG_TASKS]);
        withFrames.w.t = 1000;
        expect(withFrames.meter.tick()).toBe(85);
        resetSlowFrameTracking();
        resetBlockedTracking();
        const tasksOnly = page(["longtask"], [...BURST_FRAMES], [...BURST_LONG_TASKS]);
        tasksOnly.w.t = 1000;
        expect(tasksOnly.meter.tick()).toBe(29);
    });

    it("reads a second of frames under 50 ms as not busy, however many tasks ran", () => {
        // Forty tasks of 20 ms fill most of a second, in frames of about 25 ms each: 40 frames a second, no slow frame.
        const { w, meter } = page(["longtask", "long-animation-frame"], [], []);
        w.t = 1000;
        expect(meter.tick()).toBe(0);
    });

    it("counts a frame that was slow to draw with little script in it", () => {
        // 898 ms from first task to paint with 40 ms of script: the person waited the 898 ms.
        const { w, meter } = page(["longtask", "long-animation-frame"], [{ startTime: 50, duration: 898 }], []);
        w.t = 1000;
        expect(meter.tick()).toBe(90);
    });

    it("counts a frame that ran over the tick in the second it ended, and never over 100%", () => {
        const frames = [...backToBack([300, 300, 300], 0, 0), { startTime: 900, duration: 400 }];
        const { w, meter } = page(["longtask", "long-animation-frame"], frames, []);
        w.t = 1000;
        expect(meter.tick()).toBe(90);
        w.t = 1350;
        expect(meter.tick()).toBe(100);
    });
});
