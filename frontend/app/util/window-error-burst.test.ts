// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { BURST_COUNT, BURST_WINDOW_MS, createErrorBurst, noteWindowError, onWindowErrorBurst } from "./window-error-burst";

describe("createErrorBurst", () => {
    it("is a burst at the count within the window, and counts again from nothing after", () => {
        let t = 0;
        const burst = createErrorBurst({ count: 3, windowMs: 1000, now: () => t });
        expect([burst.note(), burst.note(), burst.note()]).toEqual([false, false, true]);
        expect([burst.note(), burst.note()]).toEqual([false, false]);
        expect(burst.note()).toBe(true);
    });

    it("does not count errors that are further apart than the window", () => {
        let t = 0;
        const burst = createErrorBurst({ count: 3, windowMs: 1000, now: () => t });
        const results: boolean[] = [];
        for (let i = 0; i < 10; i++) {
            results.push(burst.note());
            t += 600;
        }
        // Two within any 1000 ms at most: never three.
        expect(results.some(Boolean)).toBe(false);
    });

    it("counts BURST_COUNT errors inside the default window", () => {
        let t = 0;
        const burst = createErrorBurst({ now: () => t });
        const results: boolean[] = [];
        for (let i = 0; i < BURST_COUNT; i++) {
            results.push(burst.note());
            t += Math.floor(BURST_WINDOW_MS / BURST_COUNT) - 1;
        }
        expect(results.filter(Boolean)).toHaveLength(1);
        expect(results[results.length - 1]).toBe(true);
    });
});

describe("noteWindowError", () => {
    it("tells a listener at each burst, and stops when it is removed", () => {
        const heard = vi.fn();
        const stop = onWindowErrorBurst(heard);
        for (let i = 0; i < BURST_COUNT; i++) noteWindowError();
        expect(heard).toHaveBeenCalledTimes(1);
        for (let i = 0; i < BURST_COUNT; i++) noteWindowError();
        expect(heard).toHaveBeenCalledTimes(2);
        stop();
        for (let i = 0; i < BURST_COUNT; i++) noteWindowError();
        expect(heard).toHaveBeenCalledTimes(2);
    });

    it("never throws, even from a listener that does", () => {
        const stop = onWindowErrorBurst(() => {
            throw new Error("listener failed");
        });
        expect(() => {
            for (let i = 0; i < BURST_COUNT; i++) noteWindowError();
        }).not.toThrow();
        stop();
    });
});
