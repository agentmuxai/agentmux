// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { barWidth, formatGpuBytes, formatPct, GPU_WARN_PCT, gpuColor, memoryText, osName } from "./gpu-format";

const WARN = "var(--warning-text-color)";
const PLAIN = "var(--secondary-text-color)";
const GB = 1024 ** 3;
const MB = 1024 ** 2;

describe("the GPU meter's colour", () => {
    it("turns to the warning colour at 90% use or 90% memory, and not before", () => {
        expect(GPU_WARN_PCT).toBe(90);
        expect(gpuColor(89.9, 10)).toBe(PLAIN);
        expect(gpuColor(90, 10)).toBe(WARN);
        expect(gpuColor(10, 90)).toBe(WARN);
        expect(gpuColor(10, 89)).toBe(PLAIN);
    });

    it("is plain where a figure is not known", () => {
        expect(gpuColor(35, null)).toBe(PLAIN);
        expect(gpuColor(null, undefined)).toBe(PLAIN);
        expect(gpuColor(undefined, 95)).toBe(WARN);
    });
});

describe("the panel's figures", () => {
    it("writes memory in GB above a gigabyte and MB below, with one decimal under ten GB", () => {
        expect(formatGpuBytes(3.25 * GB)).toBe("3.3 GB");
        expect(formatGpuBytes(12 * GB)).toBe("12 GB");
        expect(formatGpuBytes(640 * MB)).toBe("640 MB");
        expect(formatGpuBytes(0)).toBe("0 MB");
    });

    it("writes a percentage whole", () => {
        expect(formatPct(34.6)).toBe("35%");
        expect(formatPct(0.2)).toBe("0%");
    });

    it("keeps a bar's width inside the bar", () => {
        expect(barWidth(140)).toBe(100);
        expect(barWidth(-3)).toBe(0);
        expect(barWidth(undefined)).toBe(0);
        expect(barWidth(42)).toBe(42);
    });

    it("writes memory as used of total, or used alone when the total is unknown, or nothing", () => {
        expect(memoryText(2 * GB, 8 * GB)).toBe("2.0 GB / 8.0 GB");
        expect(memoryText(2 * GB, undefined)).toBe("2.0 GB");
        expect(memoryText(2 * GB, 0)).toBe("2.0 GB");
        expect(memoryText(undefined, 8 * GB)).toBeNull();
    });
});

describe("a system by its name", () => {
    it("writes macOS, Linux and Windows as a person does, and an unknown id as it is", () => {
        expect(osName("macos")).toBe("macOS");
        expect(osName("linux")).toBe("Linux");
        expect(osName("windows")).toBe("Windows");
        expect(osName("freebsd")).toBe("freebsd");
    });
});
