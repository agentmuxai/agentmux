// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { computeAutoMaxY, formatTimeTick, getGapThresholdMs, resolveDomainBound } from "./sysinfo-util";

describe("resolveDomainBound", () => {
    it("passes a numeric bound straight through", () => {
        expect(resolveDomainBound(42, { ts: 0 })).toBe(42);
    });

    it("looks a string bound up as a field on the data item", () => {
        expect(resolveDomainBound("mem:total", { ts: 0, "mem:total": 16 })).toBe(16);
    });

    it("returns undefined for a string bound missing from the item, or an unsupported type", () => {
        expect(resolveDomainBound("mem:total", { ts: 0 })).toBeUndefined();
        expect(resolveDomainBound("mem:total", undefined as any)).toBeUndefined();
        expect(resolveDomainBound(undefined, { ts: 0 })).toBeUndefined();
    });
});

describe("getGapThresholdMs", () => {
    it("is 2.5x the interval, floored at 3000ms", () => {
        expect(getGapThresholdMs(1)).toBe(3000); // 2500 < floor
        expect(getGapThresholdMs(2)).toBe(5000); // 5000 > floor
    });

    it("treats a falsy interval as the 1.0s default", () => {
        expect(getGapThresholdMs(0)).toBe(getGapThresholdMs(1));
    });
});

describe("computeAutoMaxY", () => {
    const item = (x: number) => ({ ts: 0, x });

    it("pads the observed max by the padding fraction (default 15%)", () => {
        expect(computeAutoMaxY([item(10)], "x", 1, undefined)).toBeCloseTo(11.5);
    });

    it("never returns less than the floor, even with no data above it", () => {
        expect(computeAutoMaxY([item(0.1)], "x", 1, undefined)).toBe(1);
        expect(computeAutoMaxY([], "x", 1, undefined)).toBe(1);
    });

    it("never exceeds a hard cap, even when the padded value would clear it", () => {
        // observed 95 -> padded 109.25, but a hard cap (e.g. mem:total) must win.
        expect(computeAutoMaxY([item(95)], "x", 1, 100)).toBe(100);
    });

    it("ignores non-finite and non-numeric samples when finding the observed max", () => {
        const mixed = [item(5), { ts: 1, x: NaN }, { ts: 2, x: Infinity }, { ts: 3 }];
        expect(computeAutoMaxY(mixed, "x", 1, undefined)).toBeCloseTo(5.75);
    });

    it("honors a custom padding fraction", () => {
        expect(computeAutoMaxY([item(10)], "x", 1, undefined, 0.5)).toBeCloseTo(15);
    });
});

describe("formatTimeTick", () => {
    it("formatTimeTick shows seconds under a minute's spacing, so neighbours differ", () => {
        const t = new Date(2026, 9, 13, 8, 52, 30).getTime();
        expect(formatTimeTick(t, 30_000)).toBe("8:52:30");
        expect(formatTimeTick(t + 30_000, 30_000)).toBe("8:53:00");
        expect(formatTimeTick(new Date(2026, 9, 13, 20, 5, 0).getTime(), 60_000)).toBe("8:05 PM");
        expect(formatTimeTick(new Date(2026, 9, 13, 0, 5, 0).getTime(), 60_000)).toBe("12:05 AM");
    });
});
