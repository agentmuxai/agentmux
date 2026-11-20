// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import levels from "@/app/store/low-disk-levels.json";
import {
    classifyDisk,
    createLowDiskTracker,
    DISK_LEVELS,
    formatFreeGb,
    lowDiskText,
    readingFromSysinfo,
    type DiskReading,
} from "./low-disk";

const gb = (freeGb: number, totalGb: number): DiskReading => ({ freeGb, totalGb, freePct: (freeGb / totalGb) * 100 });

describe("the lines", () => {
    it("are the desktop's: the shared file holds what low_disk.rs warned at before it was shared", () => {
        expect(DISK_LEVELS).toEqual({ warnPct: 8, warnGb: 10, criticalPct: 4, criticalGb: 3, clearPct: 1, clearGb: 2 });
        expect(DISK_LEVELS).toBe(levels);
    });
});

// The cases of the Rust tests in crates/cef/src/memory_heartbeat/low_disk.rs, so the two cannot drift apart.
describe("classifyDisk", () => {
    it("a large drive warns on its share", () => {
        expect(classifyDisk(gb(400, 3700), "normal")).toBe("normal");
        expect(classifyDisk(gb(222, 3700), "normal")).toBe("warn");
        expect(classifyDisk(gb(140, 3700), "normal")).toBe("critical");
    });

    it("a small drive warns on gigabytes", () => {
        expect(classifyDisk(gb(9, 64), "normal")).toBe("warn");
        expect(classifyDisk(gb(2.5, 64), "normal")).toBe("critical");
        expect(classifyDisk(gb(20, 64), "normal")).toBe("normal");
    });

    it("leaving a level needs headroom over its line", () => {
        expect(classifyDisk(gb(290, 3700), "normal")).toBe("warn"); // 7.8%
        expect(classifyDisk(gb(307, 3700), "warn")).toBe("warn"); // 8.3%: over the line, inside the margin
        expect(classifyDisk(gb(340, 3700), "warn")).toBe("normal"); // 9.2%
        expect(classifyDisk(gb(160, 3700), "critical")).toBe("critical"); // 4.3%: in the margin
        expect(classifyDisk(gb(200, 3700), "critical")).toBe("warn"); // 5.4%
    });
});

describe("the tracker: once per crossing", () => {
    it("reports a crossing when the level gets worse, and not while it stays", () => {
        const t = createLowDiskTracker();
        expect(t.observe(gb(400, 3700))).toEqual({ level: "normal", crossed: false });
        expect(t.observe(gb(290, 3700))).toEqual({ level: "warn", crossed: true });
        // Every later poll at the same level, and numbers that wobble inside it, are silent.
        expect(t.observe(gb(289, 3700))).toEqual({ level: "warn", crossed: false });
        expect(t.observe(gb(307, 3700))).toEqual({ level: "warn", crossed: false });
        expect(t.observe(gb(250, 3700))).toEqual({ level: "warn", crossed: false });
    });

    it("warn to critical is a new crossing", () => {
        const t = createLowDiskTracker();
        t.observe(gb(290, 3700));
        expect(t.observe(gb(100, 3700))).toEqual({ level: "critical", crossed: true });
        expect(t.observe(gb(99, 3700))).toEqual({ level: "critical", crossed: false });
    });

    it("a recovery is not a crossing, and the next drop is one again", () => {
        const t = createLowDiskTracker();
        t.observe(gb(290, 3700));
        expect(t.observe(gb(340, 3700))).toEqual({ level: "normal", crossed: false });
        expect(t.observe(gb(341, 3700))).toEqual({ level: "normal", crossed: false });
        expect(t.observe(gb(290, 3700))).toEqual({ level: "warn", crossed: true });
    });

    it("space hovering at a line is one crossing, not one a poll", () => {
        const t = createLowDiskTracker();
        const steps = [gb(295, 3700), gb(300, 3700), gb(296, 3700), gb(303, 3700), gb(294, 3700)].map((r) => t.observe(r));
        expect(steps.filter((s) => s.crossed)).toHaveLength(1);
    });

    it("stepping down from critical to warn and falling again is a crossing", () => {
        const t = createLowDiskTracker();
        t.observe(gb(100, 3700));
        expect(t.observe(gb(200, 3700))).toEqual({ level: "warn", crossed: false });
        expect(t.observe(gb(100, 3700))).toEqual({ level: "critical", crossed: true });
    });

    it("a first reading already under a line is a crossing", () => {
        expect(createLowDiskTracker().observe(gb(2, 64))).toEqual({ level: "critical", crossed: true });
        expect(createLowDiskTracker().observe(gb(5, 64))).toEqual({ level: "warn", crossed: true });
    });
});

describe("readingFromSysinfo", () => {
    it("reads the watched volume", () => {
        expect(readingFromSysinfo({ "disk:watch:free_gb": 9.5, "disk:watch:total_gb": 100, "disk:watch:free_pct": 9.5 })).toEqual({
            freeGb: 9.5,
            totalGb: 100,
            freePct: 9.5,
        });
    });

    it("is null for a torn tick or an empty volume, never a made-up zero", () => {
        expect(readingFromSysinfo({})).toBeNull();
        expect(readingFromSysinfo({ "disk:watch:free_gb": 1, "disk:watch:total_gb": 100 })).toBeNull();
        expect(readingFromSysinfo({ "disk:watch:free_gb": 0, "disk:watch:total_gb": 0, "disk:watch:free_pct": 0 })).toBeNull();
    });
});

describe("the words", () => {
    it("say how much is free, on which volume, and how to make room", () => {
        const text = lowDiskText("warn", gb(9.5, 100), "/data");
        expect(text.title).toBe("Disk space is running low");
        expect(text.free).toBe("9.5 GB free on /data, 9.5% of 100 GB.");
        expect(text.how).toContain("Delete files");
        expect(text.how).toContain("Cloud Console");
    });

    it("name the level, and leave the volume out when it has no label", () => {
        const text = lowDiskText("critical", gb(2, 64));
        expect(text.title).toBe("Disk space is almost gone");
        expect(text.free).toBe("2.0 GB free, 3.1% of 64.0 GB.");
    });

    it("write sizes the way the desktop's notice does", () => {
        expect(formatFreeGb(9.5)).toBe("9.5 GB");
        expect(formatFreeGb(222.4)).toBe("222 GB");
    });
});
