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

    it("asks again for a gap whose fetch failed, spaced like any other", async () => {
        const calls: number[] = [];
        const log = vi.spyOn(console, "log").mockImplementation(() => {});
        let fail = true;
        const b = new GapBackfiller((from) => {
            calls.push(from);
            return fail ? Promise.reject(new Error("rpc down")) : Promise.resolve();
        }, 1000);
        b.noteGap(100);
        await vi.advanceTimersByTimeAsync(0);
        expect(calls).toEqual([100]);
        fail = false;
        await vi.advanceTimersByTimeAsync(999);
        expect(calls).toEqual([100]);
        await vi.advanceTimersByTimeAsync(1);
        expect(calls).toEqual([100, 100]);
        await vi.advanceTimersByTimeAsync(60_000);
        expect(calls).toEqual([100, 100]);
        log.mockRestore();
    });

    it("gives up on a gap after three failed tries in a row, until a new gap opens", async () => {
        const calls: number[] = [];
        const log = vi.spyOn(console, "log").mockImplementation(() => {});
        const b = new GapBackfiller((from) => {
            calls.push(from);
            return Promise.reject(new Error("rpc down"));
        }, 1000);
        b.noteGap(100);
        await vi.advanceTimersByTimeAsync(60_000);
        expect(calls).toEqual([100, 100, 100]);
        b.noteGap(500);
        await vi.advanceTimersByTimeAsync(60_000);
        expect(calls.slice(3)).toEqual([500, 500, 500]);
        log.mockRestore();
    });

    it("does not requeue a failure into a series that was reset meanwhile", async () => {
        const calls: number[] = [];
        const log = vi.spyOn(console, "log").mockImplementation(() => {});
        const rejecters: Array<(e: Error) => void> = [];
        const b = new GapBackfiller((from) => {
            calls.push(from);
            return new Promise<void>((_, reject) => rejecters.push(reject));
        }, 1000);
        b.noteGap(100);
        b.reset();
        rejecters[0](new Error("rpc down"));
        await vi.advanceTimersByTimeAsync(60_000);
        expect(calls).toEqual([100]);
        log.mockRestore();
    });

    it("uses a clock that doesn't step back for the spacing", async () => {
        const c = controlled();
        let clock = 10_000;
        const b = new GapBackfiller(c.fetchFrom, 5000, () => clock);
        b.noteGap(100);
        c.finishers[0]();
        await vi.advanceTimersByTimeAsync(0);
        // The injected clock is the one that counts, not the wall clock.
        clock = 12_000;
        b.noteGap(200);
        expect(c.calls).toEqual([100]);
        clock = 15_000;
        await vi.advanceTimersByTimeAsync(3000);
        expect(c.calls).toEqual([100, 200]);
    });

    it("forgets waiting gaps on reset, and lets later ones through", async () => {
        const c = controlled();
        const b = new GapBackfiller(c.fetchFrom, 5000);
        b.noteGap(100);
        b.noteGap(200);
        b.reset();
        c.finishers[0]();
        await vi.advanceTimersByTimeAsync(60_000);
        expect(c.calls).toEqual([100]);
        b.noteGap(300);
        await vi.advanceTimersByTimeAsync(0);
        expect(c.calls).toEqual([100, 300]);
    });

    it("by default measures the spacing on a clock a wall-clock correction can't move", async () => {
        const c = controlled();
        const b = new GapBackfiller(c.fetchFrom, 5000);
        b.noteGap(100);
        c.finishers[0]();
        await vi.advanceTimersByTimeAsync(0);
        // The system clock is corrected back by an hour.
        vi.setSystemTime(Date.now() - 3_600_000);
        b.noteGap(200);
        await vi.advanceTimersByTimeAsync(5000);
        expect(c.calls).toEqual([100, 200]);
    });

    it("is still usable after a reset, as when a pane is mounted again with the same model", async () => {
        const c = controlled();
        const b = new GapBackfiller(c.fetchFrom, 5000);
        b.noteGap(100);
        b.noteGap(200);
        b.reset();
        c.finishers[0]();
        await vi.advanceTimersByTimeAsync(60_000);
        expect(c.calls).toEqual([100]);
        b.noteGap(300);
        await vi.advanceTimersByTimeAsync(0);
        expect(c.calls).toEqual([100, 300]);
    });
});
