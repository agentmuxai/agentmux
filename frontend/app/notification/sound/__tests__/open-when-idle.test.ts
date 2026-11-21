// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The wait for the app's first frame and then for idle time (open-when-idle.ts): two frames, then an idle request
 * that gives up waiting after a timeout, or a short timer where the engine has no idle callbacks; and a cancel that
 * stops it at any point.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { IDLE_TIMEOUT_MS, NO_IDLE_API_DELAY_MS, whenIdleAfterFirstPaint } from "../open-when-idle";

let frames: Array<() => void>;
let idles: Array<{ cb: () => void; opts?: { timeout?: number } }>;
let cancelIdle: ReturnType<typeof vi.fn>;

beforeEach(() => {
    vi.useFakeTimers();
    frames = [];
    idles = [];
    cancelIdle = vi.fn();
    vi.stubGlobal("requestAnimationFrame", (fn: () => void) => frames.push(fn));
    vi.stubGlobal("requestIdleCallback", (cb: () => void, opts?: { timeout?: number }) => idles.push({ cb, opts }));
    vi.stubGlobal("cancelIdleCallback", cancelIdle);
});

afterEach(() => {
    vi.unstubAllGlobals();
    vi.useRealTimers();
});

describe("whenIdleAfterFirstPaint", () => {
    it("runs after two frames and then an idle callback, never before", () => {
        const run = vi.fn();
        whenIdleAfterFirstPaint(run);
        expect(frames).toHaveLength(1);
        expect(idles).toHaveLength(0);

        frames.shift()!();
        expect(frames).toHaveLength(1);
        expect(idles).toHaveLength(0);
        expect(run).not.toHaveBeenCalled();

        frames.shift()!();
        expect(idles).toHaveLength(1);
        expect(idles[0].opts).toEqual({ timeout: IDLE_TIMEOUT_MS });
        expect(run).not.toHaveBeenCalled();

        idles[0].cb();
        expect(run).toHaveBeenCalledTimes(1);
    });

    it("a cancel before the second frame asks for no idle time and runs nothing", () => {
        const run = vi.fn();
        const cancel = whenIdleAfterFirstPaint(run);
        frames.shift()!();
        cancel();
        frames.shift()!();
        expect(idles).toHaveLength(0);
        expect(run).not.toHaveBeenCalled();
    });

    it("a cancel while waiting for idle time withdraws the request, and a late callback runs nothing", () => {
        const run = vi.fn();
        const cancel = whenIdleAfterFirstPaint(run);
        frames.shift()!();
        frames.shift()!();
        cancel();
        expect(cancelIdle).toHaveBeenCalledTimes(1);
        idles[0].cb();
        expect(run).not.toHaveBeenCalled();
    });

    it("without idle callbacks it runs a short while after the second frame", () => {
        vi.stubGlobal("requestIdleCallback", undefined);
        const run = vi.fn();
        whenIdleAfterFirstPaint(run);
        frames.shift()!();
        frames.shift()!();
        vi.advanceTimersByTime(NO_IDLE_API_DELAY_MS - 1);
        expect(run).not.toHaveBeenCalled();
        vi.advanceTimersByTime(1);
        expect(run).toHaveBeenCalledTimes(1);
    });

    it("without animation frames it uses timers for them and still waits for idle time", () => {
        vi.stubGlobal("requestAnimationFrame", undefined);
        const run = vi.fn();
        whenIdleAfterFirstPaint(run);
        vi.advanceTimersByTime(40);
        expect(idles).toHaveLength(1);
        expect(run).not.toHaveBeenCalled();
        idles[0].cb();
        expect(run).toHaveBeenCalledTimes(1);
    });
});
