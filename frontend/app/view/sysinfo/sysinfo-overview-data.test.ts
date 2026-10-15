// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import {
    BUSIEST_CORE,
    columnSpan,
    coreGrid,
    coreKeys,
    formatGB,
    formatRate,
    mirroredRange,
    toAreaChoice,
    formatWindow,
    windowChoices,
    volumes,
    windowStats,
    withBusiestCore,
} from "./sysinfo-overview-data";
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

describe("an area alone", () => {
    it("reads a saved area, and anything else as CPU", () => {
        expect(toAreaChoice("net")).toBe("net");
        expect(toAreaChoice("everything")).toBe("everything");
        // Nothing saved, or a value from an older build, shows CPU.
        expect(toAreaChoice(undefined)).toBe("cpu");
        expect(toAreaChoice("overview")).toBe("cpu");
        expect(toAreaChoice(3)).toBe("cpu");
    });

    it("adds each sample's busiest core, and keeps a gap a gap", () => {
        const rows = [row(1, { "cpu:0": 10, "cpu:1": 90 }), marker(2), row(3, { "cpu:0": 50 })];
        const out = withBusiestCore(rows, ["cpu:0", "cpu:1"]);
        expect(out.map((r) => r[BUSIEST_CORE])).toEqual([90, NaN, 50]);
        expect(out[1].blank).toBe(1);
        expect(withBusiestCore(rows, [])).toBe(rows);
    });

    it("lists the volumes a sample carries, a Windows mount with its colon included", () => {
        const win = "disk:vol:C:" + String.fromCharCode(92);
        const r = row(1, {
            [`${win}:free_gb`]: 100,
            [`${win}:total_gb`]: 500,
            [`${win}:is_system_drive`]: 1,
            "disk:vol:/data:free_gb": 5,
            "disk:vol:/data:total_gb": 0,
        });
        // A volume without a size is left out.
        expect(volumes(r)).toEqual([{ mount: "C:" + String.fromCharCode(92), freeGB: 100, totalGB: 500 }]);
        expect(volumes(undefined)).toEqual([]);
    });
});

describe("columnSpan", () => {
    const win: [number, number] = [100, 160]; // a 60 s window, as the charts take it

    it("places a sample by its time, covering the interval that ends at it", () => {
        expect(columnSpan(130_000, win, 1, 600)).toEqual([290, 300]);
        expect(columnSpan(160_000, win, 1, 600)).toEqual([590, 600]);
    });

    it("leaves the start of the window empty when there are fewer samples than it holds", () => {
        // Two samples at the end of a 60 s window sit at its right edge, not across it.
        expect(columnSpan(159_000, win, 1, 600)?.[0]).toBe(580);
    });

    it("clips at the window's start, and has nothing outside it", () => {
        expect(columnSpan(100_500, win, 1, 600)).toEqual([0, 5]);
        expect(columnSpan(90_000, win, 1, 600)).toBeNull();
        expect(columnSpan(130_000, [100, 100], 1, 600)).toBeNull();
    });
});

describe("windowChoices", () => {
    it("spans the labelled time at the sample interval", () => {
        expect(windowChoices(1).map((c) => c.points)).toEqual([60, 120, 300, 900]);
        expect(windowChoices(2)).toEqual([
            { secs: 60, points: 30, label: "1m" },
            { secs: 120, points: 60, label: "2m" },
            { secs: 300, points: 150, label: "5m" },
            { secs: 900, points: 450, label: "15m" },
        ]);
    });

    it("leaves out a window longer than srv's history holds", () => {
        expect(windowChoices(0.5).map((c) => c.label)).toEqual(["1m", "2m", "5m"]);
    });
});

describe("a window set as a sample count", () => {
    it("shows as a button of its own when it is none of the choices", () => {
        // 120 samples at 2 s: four minutes.
        expect(windowChoices(2, 240).map((c) => [c.label, c.secs])).toEqual([
            ["1m", 60],
            ["2m", 120],
            ["4m", 240],
            ["5m", 300],
            ["15m", 900],
        ]);
        expect(windowChoices(1, 300)).toHaveLength(4);
    });

    it("reads in seconds or minutes", () => {
        expect([formatWindow(45), formatWindow(240), formatWindow(150)]).toEqual(["45s", "4m", "2.5m"]);
    });
});
