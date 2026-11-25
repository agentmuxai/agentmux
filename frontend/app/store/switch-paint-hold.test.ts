// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
    afterSwitchPaint,
    dirtyAtClick,
    HOLD_CAP_MS,
    holdForSwitchPaint,
    noteSwitchActivity,
    releaseSwitchPaint,
    resetSwitchPaintHold,
    switchPaintHeld,
    waitingAfterSwitchPaint,
} from "./switch-paint-hold";

describe("switch-paint-hold", () => {
    beforeEach(() => {
        vi.useFakeTimers();
        resetSwitchPaintHold();
    });
    afterEach(() => {
        resetSwitchPaintHold();
        vi.useRealTimers();
    });

    it("runs work at once when no switch is in flight", () => {
        const fn = vi.fn();
        afterSwitchPaint(fn);
        expect(fn).toHaveBeenCalledTimes(1);
    });

    it("queues work during a switch and runs it, in order, after the release", () => {
        const order: number[] = [];
        const token = holdForSwitchPaint();
        afterSwitchPaint(() => order.push(1));
        afterSwitchPaint(() => order.push(2));
        expect(order).toEqual([]);
        expect(waitingAfterSwitchPaint()).toBe(2);
        expect(releaseSwitchPaint(token)).toBe(2);
        expect(order).toEqual([]);
        vi.advanceTimersByTime(1);
        expect(order).toEqual([1, 2]);
        expect(switchPaintHeld()).toBe(false);
    });

    it("lets work run at the cap when the release never comes", () => {
        const fn = vi.fn();
        holdForSwitchPaint();
        afterSwitchPaint(fn);
        vi.advanceTimersByTime(HOLD_CAP_MS + 1);
        expect(fn).toHaveBeenCalledTimes(1);
        expect(switchPaintHeld()).toBe(false);
    });

    it("ignores the release of an older switch once a newer one holds", () => {
        const fn = vi.fn();
        const first = holdForSwitchPaint();
        const second = holdForSwitchPaint();
        afterSwitchPaint(fn);
        expect(releaseSwitchPaint(first)).toBe(0);
        expect(switchPaintHeld()).toBe(true);
        releaseSwitchPaint(second);
        vi.advanceTimersByTime(1);
        expect(fn).toHaveBeenCalledTimes(1);
    });

    it("counts what arrived in the second before the click, most frequent first", () => {
        noteSwitchActivity("waveobj:update", 300);
        noteSwitchActivity("waveobj:update", 400);
        noteSwitchActivity("blockfile", 500);
        noteSwitchActivity("vault_op", 50);
        noteSwitchActivity("sysinfo", 1700);
        const dirty = dirtyAtClick(1200);
        expect(dirty.total).toBe(3);
        expect(dirty.mix).toBe("waveobj:update:2,blockfile:1");
    });
});
