// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { nextSharedZoneDelay, startCadencedPoll, type PollOutcome } from "./shared-zone-cadence";

beforeEach(() => {
    vi.useFakeTimers();
});
afterEach(() => vi.useRealTimers());

describe("nextSharedZoneDelay", () => {
    it("is five seconds while the count moves, then doubles up to thirty", () => {
        expect([0, 1, 2, 3, 4, 10].map(nextSharedZoneDelay)).toEqual([5_000, 10_000, 20_000, 30_000, 30_000, 30_000]);
    });
});

describe("startCadencedPoll", () => {
    it("never has two checks at once, however long one takes", async () => {
        let running = 0;
        let peak = 0;
        const stop = startCadencedPoll(async () => {
            running += 1;
            peak = Math.max(peak, running);
            await new Promise((r) => setTimeout(r, 12_000));
            running -= 1;
            return "moved";
        });
        await vi.advanceTimersByTimeAsync(60_000);
        stop();
        expect(peak).toBe(1);
    });

    it("counts a check that throws, or learns nothing, as no change in pace", async () => {
        const outcomes: Array<PollOutcome | "throw"> = ["still", "throw", "skipped", "still"];
        const at: number[] = [];
        const stop = startCadencedPoll(async () => {
            at.push(Date.now());
            const next = outcomes.shift();
            if (next === "throw") throw new Error("down");
            return next ?? "moved";
        });
        const t0 = Date.now();
        await vi.advanceTimersByTimeAsync(80_000);
        stop();
        // 5 s, then one still check: 10 s, a throw and a skip leave it at 10 s, one more still: 20 s.
        expect(at.slice(0, 5).map((t) => (t - t0) / 1000)).toEqual([5, 15, 25, 35, 55]);
    });

    it("stops when stopped, also from inside a check", async () => {
        let calls = 0;
        let stop = () => {};
        stop = startCadencedPoll(async () => {
            calls += 1;
            stop();
            return "moved";
        });
        await vi.advanceTimersByTimeAsync(60_000);
        expect(calls).toBe(1);
    });
});
