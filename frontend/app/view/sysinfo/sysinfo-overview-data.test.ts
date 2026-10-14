// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { coreGrid, coreKeys, formatGB, formatRate, mirroredRange, windowStats } from "./sysinfo-overview-data";
import type { DataItem } from "./sysinfo-types";

const row = (ts: number, v: Record<string, number>): DataItem => ({ ts, ...v });
const marker = (ts: number): DataItem => ({ ts, cpu: NaN, blank: 1 });

describe("windowStats", () => {
    it("gives the newest reading and the window's low, high and average, skipping gaps", () => {
        const rows = [row(1, { cpu: 10 }), marker(2), row(3, { cpu: 30 }), row(4, { cpu: 20 }), row(5, {})];
        expect(windowStats(rows, "cpu")).toEqual({ now: 20, min: 10, max: 30, avg: 20 });
    });

    it("is null with no reading at all", () => {
        expect(windowStats([marker(1), row(2, {})], "cpu")).toBeNull();
        expect(windowStats([], "cpu")).toBeNull();
    });
});

describe("cores", () => {
    it("coreKeys lists the per-core keys in core order, not text order", () => {
        expect(coreKeys(row(1, { cpu: 5, "cpu:10": 1, "cpu:2": 1, "cpu:0": 1, "mem:used": 3 }))).toEqual([
            "cpu:0",
            "cpu:2",
            "cpu:10",
        ]);
        expect(coreKeys(undefined)).toEqual([]);
    });

    it("coreGrid has a row per core and a column per sample, null at a gap", () => {
        const rows = [row(1, { "cpu:0": 10, "cpu:1": 90 }), marker(2), row(3, { "cpu:0": 20 })];
        expect(coreGrid(rows, ["cpu:0", "cpu:1"])).toEqual([
            [10, null, 20],
            [90, null, null],
        ]);
    });
});

describe("mirroredRange", () => {
    it("is symmetric around zero, on the larger of the two, rounded up", () => {
        const rows = [row(1, { recv: 3.2, sent: 0.5 }), row(2, { recv: 1, sent: 4.1 })];
        expect(mirroredRange(rows, "recv", "sent", 1)).toEqual([-5, 5]);
    });

    it("never goes under its floor on a quiet link", () => {
        expect(mirroredRange([row(1, { recv: 0.01, sent: 0 })], "recv", "sent", 1)).toEqual([-1, 1]);
    });
});

describe("formatting", () => {
    it("formatRate picks KB/s, MB/s or GB/s", () => {
        expect([0.05, 0.9994, 1, 12.34, 1500, null].map(formatRate)).toEqual([
            "50 KB/s",
            "999 KB/s",
            "1.0 MB/s",
            "12.3 MB/s",
            "1.5 GB/s",
            "–",
        ]);
    });

    it("formatGB rounds to one decimal", () => {
        expect(formatGB(12.345)).toBe("12.3 GB");
        expect(formatGB(undefined)).toBe("–");
    });
});
