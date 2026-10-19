// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { segmentsOf, slowSendBreakdown } from "./lib/sendsegments.mjs";

describe("where in the page a slow send's time went", () => {
    it("splits the page's time at the moment the answer reached its JS", () => {
        // the lab started at 1,000; JS ran at 1,400 for 28,000 ms; the answer reached the page's handler at 28,400
        expect(segmentsOf({ start: 1000, ms: 29000, pageStart: 1400, pageMs: 28000, respArrival: 28400 })).toMatchObject({ before: 400, inPage: 28000, toArrival: 27000, afterArrival: 1000 });
        // no stamp: the split is unknown, never zero
        expect(segmentsOf({ start: 0, ms: 6000, pageStart: 10, pageMs: 5900 })).toMatchObject({ toArrival: null, afterArrival: null });
        // clock noise never gives a negative figure
        expect(segmentsOf({ start: 0, ms: 100, pageStart: 10, pageMs: 80, respArrival: 5 })).toMatchObject({ toArrival: 0 });
    });

    it("counts the slow sends by which half of the page's time held more", () => {
        const sends = [
            { start: 0, ms: 9000, pageStart: 0, pageMs: 9000, respArrival: 8500 }, // the answer reached JS late
            { start: 0, ms: 9000, pageStart: 0, pageMs: 9000, respArrival: 500 }, // JS had it early, resolving took the time
            { start: 0, ms: 7000, pageStart: 0, pageMs: 7000 }, // no stamp
            { start: 0, ms: 300, pageStart: 0, pageMs: 280, respArrival: 250 }, // not slow
        ];
        const b = slowSendBreakdown(sends);
        expect(b.inPageSplit).toEqual({ toArrival: 1, afterArrival: 1, unknown: 1 });
        expect(b.maxToArrivalMs).toBe(8500);
        expect(b.maxAfterArrivalMs).toBe(8500);
    });
});
