// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { Aggregator, KeyedAggregator } from "./aggregates";

describe("Aggregator", () => {
    it("returns zeroes when empty", () => {
        const a = new Aggregator(8);
        expect(a.quantiles()).toEqual({ count: 0, p50: 0, p75: 0, p95: 0, max: 0 });
    });

    it("computes quantiles on a partially-filled buffer", () => {
        const a = new Aggregator(100);
        for (let i = 1; i <= 10; i++) a.record(i);
        const q = a.quantiles();
        expect(q.count).toBe(10);
        expect(q.p50).toBe(6); // Math.floor(0.5 * 10) = 5 → sorted[5] = 6
        expect(q.max).toBe(10);
    });

    it("rolls over the ring buffer when full", () => {
        const a = new Aggregator(4);
        a.record(1);
        a.record(2);
        a.record(3);
        a.record(4);
        a.record(100); // overwrites slot 0
        const q = a.quantiles();
        expect(q.count).toBe(4);
        expect(q.max).toBe(100);
    });

    it("p95 lands at the top of the distribution", () => {
        const a = new Aggregator(100);
        for (let i = 1; i <= 100; i++) a.record(i);
        const q = a.quantiles();
        // P95 of [1..100]: floor(0.95 * 100) = 95 → sorted[95] = 96
        expect(q.p95).toBe(96);
    });

    it("reset clears the buffer", () => {
        const a = new Aggregator(4);
        a.record(1);
        a.record(2);
        a.reset();
        expect(a.quantiles().count).toBe(0);
    });
});

describe("KeyedAggregator", () => {
    it("records per-key values independently", () => {
        const k = new KeyedAggregator(64);
        k.record("a", 10);
        k.record("a", 20);
        k.record("b", 100);
        const snap = k.snapshot();
        expect(snap.get("a")?.count).toBe(2);
        expect(snap.get("b")?.count).toBe(1);
        expect(snap.get("b")?.max).toBe(100);
    });

    it("topByP95 ranks descending", () => {
        const k = new KeyedAggregator(64);
        for (let i = 0; i < 20; i++) {
            k.record("slow", 100);
            k.record("medium", 50);
            k.record("fast", 5);
        }
        const top = k.topByP95(2);
        expect(top.length).toBe(2);
        expect(top[0].key).toBe("slow");
        expect(top[1].key).toBe("medium");
    });
});
