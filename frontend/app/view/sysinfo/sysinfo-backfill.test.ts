// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { GapBackfiller } from "./sysinfo-backfill";

beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(1_000_000);
});

afterEach(() => {
    vi.useRealTimers();
});

/** A fetch whose finish the test controls. */
function controlled() {
    const calls: number[] = [];
    const finishers: Array<() => void> = [];
    const fetchFrom = (from: number) => {
        calls.push(from);
        return new Promise<void>((resolve) => finishers.push(resolve));
    };
    return { calls, finishers, fetchFrom };
}

describe("GapBackfiller", () => {
    it("fetches at once for the first gap", () => {
        const c = controlled();
        new GapBackfiller(c.fetchFrom, 5000).noteGap(900);
        expect(c.calls).toEqual([900]);
    });

    it("runs one fetch at a time and fetches again from the earliest gap noted meanwhile", async () => {
        const c = controlled();
        const b = new GapBackfiller(c.fetchFrom, 5000);
        b.noteGap(900);
        b.noteGap(950);
        b.noteGap(920);
        expect(c.calls).toEqual([900]);
        c.finishers[0]();
        await vi.advanceTimersByTimeAsync(0);
        // Not before the minimum spacing.
        expect(c.calls).toEqual([900]);
        await vi.advanceTimersByTimeAsync(5000);
        expect(c.calls).toEqual([900, 920]);
    });

    it("spaces fetches by the minimum interval even when each finishes at once", async () => {
        const c = controlled();
        const b = new GapBackfiller(c.fetchFrom, 5000);
        b.noteGap(100);
        c.finishers[0]();
        await vi.advanceTimersByTimeAsync(1000);
        b.noteGap(200);
        expect(c.calls).toEqual([100]);
        await vi.advanceTimersByTimeAsync(3999);
        expect(c.calls).toEqual([100]);
        await vi.advanceTimersByTimeAsync(1);
        expect(c.calls).toEqual([100, 200]);
    });

    it("does nothing more once there is no gap left to fetch", async () => {
        const c = controlled();
        const b = new GapBackfiller(c.fetchFrom, 5000);
        b.noteGap(100);
        c.finishers[0]();
        await vi.advanceTimersByTimeAsync(60_000);
        expect(c.calls).toEqual([100]);
    });

    it("keeps going after a fetch fails", async () => {
        const calls: number[] = [];
        const log = vi.spyOn(console, "log").mockImplementation(() => {});
        const b = new GapBackfiller((from) => {
            calls.push(from);
            return Promise.reject(new Error("rpc down"));
        }, 1000);
        b.noteGap(100);
        await vi.advanceTimersByTimeAsync(0);
        b.noteGap(300);
        await vi.advanceTimersByTimeAsync(1000);
        expect(calls).toEqual([100, 300]);
        log.mockRestore();
    });

    it("stops after dispose", async () => {
        const c = controlled();
        const b = new GapBackfiller(c.fetchFrom, 5000);
        b.noteGap(100);
        b.noteGap(200);
        b.dispose();
        c.finishers[0]();
        await vi.advanceTimersByTimeAsync(60_000);
        expect(c.calls).toEqual([100]);
        b.noteGap(300);
        expect(c.calls).toEqual([100]);
    });
});
