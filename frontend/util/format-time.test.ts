// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { formatElapsedClock, formatElapsedCompact, formatExactTime, formatTimeAgo } from "./format-time";

describe("formatElapsedCompact", () => {
    it("renders seconds only under a minute", () => {
        expect(formatElapsedCompact(0)).toBe("0s");
        expect(formatElapsedCompact(999)).toBe("0s");
        expect(formatElapsedCompact(1_000)).toBe("1s");
        expect(formatElapsedCompact(42_000)).toBe("42s");
        expect(formatElapsedCompact(59_000)).toBe("59s");
    });

    it("renders minutes + seconds at/above a minute", () => {
        expect(formatElapsedCompact(60_000)).toBe("1m 0s");
        expect(formatElapsedCompact(185_000)).toBe("3m 5s");
    });

    it("floors negative durations to 0s", () => {
        expect(formatElapsedCompact(-500)).toBe("0s");
    });
});

describe("formatElapsedClock", () => {
    it("renders mm:ss, zero-padded", () => {
        expect(formatElapsedClock(0)).toBe("0:00");
        expect(formatElapsedClock(5_000)).toBe("0:05");
        expect(formatElapsedClock(185_000)).toBe("3:05");
        expect(formatElapsedClock(600_000)).toBe("10:00");
    });

    it("floors negative durations to 0:00 (fixes the PersistentShellBlock gap)", () => {
        expect(formatElapsedClock(-65_000)).toBe("0:00");
    });
});

describe("formatTimeAgo", () => {
    it("returns 'just now' for sub-minute deltas", () => {
        expect(formatTimeAgo(1_000_000 - 30_000, 1_000_000)).toBe("just now");
    });
    it("returns 'Xm ago' for sub-hour deltas", () => {
        expect(formatTimeAgo(10_000_000 - 5 * 60_000, 10_000_000)).toBe("5m ago");
    });
    it("returns 'Xh ago' for sub-day deltas", () => {
        expect(formatTimeAgo(100_000_000 - 3 * 3_600_000, 100_000_000)).toBe("3h ago");
    });
    it("returns 'Xd ago' for multi-day deltas", () => {
        expect(formatTimeAgo(1_000_000_000 - 2 * 86_400_000, 1_000_000_000)).toBe("2d ago");
    });
    it("returns '' for zero (falsy ms)", () => {
        expect(formatTimeAgo(0, Date.now())).toBe("");
    });
    it("defaults `now` to Date.now() when omitted", () => {
        expect(formatTimeAgo(Date.now() - 5_000)).toBe("just now");
    });
});

describe("formatExactTime", () => {
    // 2026-08-30: switched from a 24-hour clock to 12-hour with AM/PM at
    // operator request. Local time throughout — `getHours()` always was.
    it("renders local 12-hour time with an AM suffix, minutes/seconds zero-padded", () => {
        const d = new Date(2026, 0, 1, 9, 5, 3);
        expect(formatExactTime(d.getTime())).toBe("9:05:03 AM");
    });
    it("renders afternoon times as PM on a 12-hour clock", () => {
        const d = new Date(2026, 0, 1, 14, 32, 7);
        expect(formatExactTime(d.getTime())).toBe("2:32:07 PM");
    });
    it("midnight is 12 AM, not 0 AM", () => {
        const d = new Date(2026, 0, 1, 0, 7, 9);
        expect(formatExactTime(d.getTime())).toBe("12:07:09 AM");
    });
    it("noon is 12 PM, not 0 PM", () => {
        const d = new Date(2026, 0, 1, 12, 0, 0);
        expect(formatExactTime(d.getTime())).toBe("12:00:00 PM");
    });
    it("does not zero-pad the hour", () => {
        const d = new Date(2026, 0, 1, 15, 4, 5);
        expect(formatExactTime(d.getTime())).toBe("3:04:05 PM");
    });
});
