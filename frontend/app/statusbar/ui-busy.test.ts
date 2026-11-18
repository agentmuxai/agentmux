// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { busyPercent, createUiBusyMeter, UI_BUSY_WARN_PCT, uiBusyColor } from "./ui-busy";

describe("the UI busy share", () => {
    it("is the long-task time over the window, whole and within 0 to 100", () => {
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

/** A clock and a page that can be hidden, with the long-task time the page has had in each window. */
function world(blocked: (start: number, end: number) => number | null = () => 0) {
    const w = { t: 0, visible: true };
    const meter = createUiBusyMeter({ now: () => w.t, visible: () => w.visible, blockedBetween: blocked });
    return { w, meter };
}

describe("the UI busy meter", () => {
    it("says nothing on its first tick: there is no second to measure yet", () => {
        const { meter } = world();
        expect(meter.tick()).toBeNull();
    });

    it("reads each second's long tasks as a share of that second", () => {
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

    it("shows nothing where the engine has no Long Tasks API", () => {
        const { w, meter } = world(() => null);
        meter.tick();
        w.t = 1000;
        expect(meter.tick()).toBeNull();
    });
});
