// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { formatCompactNumber, formatExactNumber } from "./format-count";

describe("formatCompactNumber", () => {
    it("returns raw digits below 1000", () => {
        expect(formatCompactNumber(0)).toBe("0");
        expect(formatCompactNumber(1)).toBe("1");
        expect(formatCompactNumber(999)).toBe("999");
    });

    it("uses one decimal place below 10x the k tier", () => {
        expect(formatCompactNumber(1_000)).toBe("1.0k");
        expect(formatCompactNumber(1_200)).toBe("1.2k");
        expect(formatCompactNumber(9_949)).toBe("9.9k");
    });

    it("can display as the next tier's rounded value while still inside this tier (pre-existing quirk, not a regression)", () => {
        // 9960/1000 = 9.96 -> toFixed(1) rounds to "10.0" while still inside
        // the k tier (raw value 9960 < 10_000) — matches the behavior the
        // two pre-existing tiered implementations already had.
        expect(formatCompactNumber(9_960)).toBe("10.0k");
    });

    it("uses integer precision at 10x the k tier and above", () => {
        expect(formatCompactNumber(10_000)).toBe("10k");
        expect(formatCompactNumber(45_000)).toBe("45k");
        expect(formatCompactNumber(999_000)).toBe("999k");
    });

    it("rolls over to m at exactly 1,000,000", () => {
        expect(formatCompactNumber(999_999)).toBe("1000k");
        expect(formatCompactNumber(1_000_000)).toBe("1.0m");
        expect(formatCompactNumber(1_200_000)).toBe("1.2m");
        // 12,345,678 is >= 10x the m tier (10,000,000), so it falls in the
        // integer-precision sub-range, same rule as the k tier at >= 10,000.
        expect(formatCompactNumber(12_345_678)).toBe("12m");
    });

    it("uses integer precision at 10x the m tier and above", () => {
        expect(formatCompactNumber(10_000_000)).toBe("10m");
        expect(formatCompactNumber(999_000_000)).toBe("999m");
    });

    it("rolls over to b at exactly 1,000,000,000", () => {
        expect(formatCompactNumber(999_999_999)).toBe("1000m");
        expect(formatCompactNumber(1_000_000_000)).toBe("1.0b");
        expect(formatCompactNumber(1_200_000_000)).toBe("1.2b");
    });

    it("handles negative numbers defensively (no current call site produces them)", () => {
        expect(formatCompactNumber(-500)).toBe("-500");
        expect(formatCompactNumber(-1_200)).toBe("-1.2k");
        expect(formatCompactNumber(-1_200_000)).toBe("-1.2m");
    });
});

describe("formatExactNumber", () => {
    it("comma-groups thousands", () => {
        expect(formatExactNumber(0)).toBe("0");
        expect(formatExactNumber(999)).toBe("999");
        expect(formatExactNumber(1_000)).toBe("1,000");
        expect(formatExactNumber(1_234_567)).toBe("1,234,567");
    });
});
