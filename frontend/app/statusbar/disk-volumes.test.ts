// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { diskFreeColor, diskTooltip, formatDiskGb, parseDiskVolumes } from "./disk-volumes";

describe("parseDiskVolumes", () => {
    it("parses Windows mounts whose own colons/backslashes sit inside the key", () => {
        const vols = parseDiskVolumes({
            "disk:vol:C:\\:free_gb": 120.5,
            "disk:vol:C:\\:total_gb": 460.2,
            "disk:vol:D:\\:free_gb": 900,
            "disk:vol:D:\\:total_gb": 1000,
        });
        expect(vols).toEqual([
            { label: "C:", freeGb: 120.5, totalGb: 460.2, isWatch: false, isSystemDrive: false },
            { label: "D:", freeGb: 900, totalGb: 1000, isWatch: false, isSystemDrive: false },
        ]);
    });

    it("parses unix-style mounts, keeping bare '/' intact", () => {
        const vols = parseDiskVolumes({
            "disk:vol:/:free_gb": 40,
            "disk:vol:/:total_gb": 100,
            "disk:vol:/home:free_gb": 10,
            "disk:vol:/home:total_gb": 50,
        });
        expect(vols.map((v) => v.label)).toEqual(["/", "/home"]);
    });

    it("flags the watch volume and ignores unrelated sysinfo keys", () => {
        const vols = parseDiskVolumes({
            cpu: 12,
            "disk:read": 1.5,
            "disk:pagefile_volume:free_pct": 26,
            "disk:vol:C:\\:free_gb": 120,
            "disk:vol:C:\\:total_gb": 460,
            "disk:vol:C:\\:watch": 1,
            "disk:vol:D:\\:free_gb": 900,
            "disk:vol:D:\\:total_gb": 1000,
        });
        expect(vols.find((v) => v.label === "C:")?.isWatch).toBe(true);
        expect(vols.find((v) => v.label === "D:")?.isWatch).toBe(false);
    });

    it("flags the system-drive volume separately from the watch volume", () => {
        // Custom page-file placement on a non-system drive: E: is watched
        // (has the page file) but C: is the actual %SystemDrive%.
        const vols = parseDiskVolumes({
            "disk:vol:C:\\:free_gb": 120,
            "disk:vol:C:\\:total_gb": 460,
            "disk:vol:E:\\:free_gb": 900,
            "disk:vol:E:\\:total_gb": 1000,
            "disk:vol:E:\\:watch": 1,
            "disk:vol:E:\\:is_system_drive": 0,
        });
        const e = vols.find((v) => v.label === "E:");
        expect(e?.isWatch).toBe(true);
        expect(e?.isSystemDrive).toBe(false);
        expect(vols.find((v) => v.label === "C:")?.isSystemDrive).toBe(false); // no explicit key -> defaults false
    });

    it("drops a volume missing either side of the pair instead of fabricating a 0", () => {
        const vols = parseDiskVolumes({
            "disk:vol:C:\\:free_gb": 120,
            // total missing — torn/partial tick
            "disk:vol:D:\\:total_gb": 1000,
            // free missing
        });
        expect(vols).toEqual([]);
    });

    it("sorts by label", () => {
        const vols = parseDiskVolumes({
            "disk:vol:E:\\:free_gb": 1,
            "disk:vol:E:\\:total_gb": 2,
            "disk:vol:C:\\:free_gb": 1,
            "disk:vol:C:\\:total_gb": 2,
        });
        expect(vols.map((v) => v.label)).toEqual(["C:", "E:"]);
    });
});

describe("formatDiskGb", () => {
    it("formats terabytes, gigabytes, and megabytes", () => {
        expect(formatDiskGb(1536)).toBe("1.5T");
        expect(formatDiskGb(320.44)).toBe("320.4G");
        expect(formatDiskGb(0.5)).toBe("512M");
    });
});

describe("diskTooltip", () => {
    it("matches the short stat-tooltip format, naming the drive, never mentioning the page file", () => {
        const tip = diskTooltip([
            { label: "C:", freeGb: 120.5, totalGb: 460.2, isWatch: true, isSystemDrive: true },
            { label: "D:", freeGb: 900, totalGb: 1000, isWatch: false, isSystemDrive: false },
        ]);
        expect(tip).toBe("Free share of system drive (C:)");
        expect(tip.toLowerCase()).not.toContain("page");
        expect(tip).not.toContain("PF");
    });

    it("falls back to the driveless form when no watch volume is known yet", () => {
        expect(diskTooltip([])).toBe("Free share of system drive");
    });

    it("names the drive plainly, without claiming it's the system drive, when the page file lives elsewhere", () => {
        // reagent finding on #2479: don't say "system drive (E:)" when E:
        // isn't actually %SystemDrive% -- self-contradictory.
        const tip = diskTooltip([
            { label: "C:", freeGb: 50, totalGb: 460, isWatch: false, isSystemDrive: true },
            { label: "E:", freeGb: 900, totalGb: 1000, isWatch: true, isSystemDrive: false },
        ]);
        expect(tip).toBe("Free share of E:");
        expect(tip).not.toContain("system drive");
        expect(tip.toLowerCase()).not.toContain("page");
    });
});

describe("diskFreeColor", () => {
    it("brackets match the pill's own thresholds: <8% error, <15% warning", () => {
        expect(diskFreeColor(7, 100)).toBe("var(--error-color)");
        expect(diskFreeColor(14, 100)).toBe("var(--warning-color)");
        expect(diskFreeColor(50, 100)).toBe("var(--secondary-text-color)");
        expect(diskFreeColor(1, 0)).toBe("var(--secondary-text-color)"); // no capacity → no judgment
    });
});
