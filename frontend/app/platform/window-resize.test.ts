// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * window-resize.ts — the "window is being resized" signal that lets hidden
 * window tabs skip layout during a resize, and the idle drain that lets them
 * catch up afterwards (ANALYSIS_WINDOW_RESIZE_REPAINT_LAG_2026_10_06.md).
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import {
    drainWhenIdle,
    installWindowResizeTracking,
    noteWindowResize,
    resetWindowResizeForTests,
    WINDOW_RESIZE_SETTLE_MS,
    windowResizing,
} from "./window-resize";

afterEach(() => {
    resetWindowResizeForTests();
    vi.useRealTimers();
});

describe("windowResizing", () => {
    it("is on from the first resize until the window settles", () => {
        vi.useFakeTimers();
        expect(windowResizing()).toBe(false);
        noteWindowResize();
        expect(windowResizing()).toBe(true);
        vi.advanceTimersByTime(WINDOW_RESIZE_SETTLE_MS - 1);
        expect(windowResizing()).toBe(true);
        vi.advanceTimersByTime(1);
        expect(windowResizing()).toBe(false);
    });

    it("stays on through a drag: each step pushes the settle point back", () => {
        vi.useFakeTimers();
        for (let i = 0; i < 10; i++) {
            noteWindowResize();
            vi.advanceTimersByTime(16);
        }
        expect(windowResizing()).toBe(true);
        vi.advanceTimersByTime(WINDOW_RESIZE_SETTLE_MS);
        expect(windowResizing()).toBe(false);
    });

    it("follows the window's resize events once installed, and installs once", () => {
        vi.useFakeTimers();
        const add = vi.spyOn(window, "addEventListener");
        installWindowResizeTracking(window);
        installWindowResizeTracking(window);
        expect(add.mock.calls.filter(([type]) => type === "resize")).toHaveLength(1);
        window.dispatchEvent(new Event("resize"));
        expect(windowResizing()).toBe(true);
        add.mockRestore();
    });
});

describe("drainWhenIdle", () => {
    it("runs one step per idle period until a step returns false", () => {
        vi.useFakeTimers();
        vi.stubGlobal("requestIdleCallback", undefined);
        let left = 3;
        const step = vi.fn(() => --left > 0);
        drainWhenIdle(step);
        expect(step).not.toHaveBeenCalled();
        vi.advanceTimersByTime(16);
        expect(step).toHaveBeenCalledTimes(1);
        vi.advanceTimersByTime(100);
        expect(step).toHaveBeenCalledTimes(3);
        vi.unstubAllGlobals();
    });

    it("stops when cancelled", () => {
        vi.useFakeTimers();
        vi.stubGlobal("requestIdleCallback", undefined);
        const step = vi.fn(() => true);
        const cancel = drainWhenIdle(step);
        vi.advanceTimersByTime(16);
        cancel();
        vi.advanceTimersByTime(200);
        expect(step).toHaveBeenCalledTimes(1);
        vi.unstubAllGlobals();
    });
});
