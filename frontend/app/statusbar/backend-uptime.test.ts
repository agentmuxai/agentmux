// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { formatUptime, resolveUptimeSecs } from "./backend-uptime";

describe("formatUptime", () => {
    it("formats sub-hour uptimes as m:ss", () => {
        expect(formatUptime(0)).toBe("0:00");
        expect(formatUptime(9)).toBe("0:09");
        expect(formatUptime(75)).toBe("1:15");
        expect(formatUptime(3599)).toBe("59:59");
    });

    it("formats sub-day uptimes as h:mm:ss", () => {
        expect(formatUptime(3600)).toBe("1:00:00");
        expect(formatUptime(45296)).toBe("12:34:56");
    });

    it("formats multi-day uptimes as d:hh:mm:ss", () => {
        expect(formatUptime(86400)).toBe("1:00:00:00");
        expect(formatUptime(90061)).toBe("1:01:01:01");
    });

    // Regression: a backwards system-clock step (NTP correction, manual set,
    // VM resume) used to drive uptimeSecs negative, and neither formatUptime
    // nor its pad2 helper guarded the sign — `secs % 60` stayed negative and
    // pad2's `n < 10 ? "0" + n` prepended a zero to the MINUS SIGN, rendering
    // e.g. "-59:0-14" in the status bar. Worse, it read as a countdown: as ts
    // advanced the seconds field went "0-14" -> "0-13" -> "0-12".
    it("clamps negative uptimes to zero rather than rendering a padded minus sign", () => {
        expect(formatUptime(-14)).toBe("0:00");
        expect(formatUptime(-1717984694)).toBe("0:00");
    });

    it("clamps non-finite input to zero", () => {
        expect(formatUptime(NaN)).toBe("0:00");
        expect(formatUptime(Infinity)).toBe("0:00");
    });

    it("truncates fractional seconds", () => {
        expect(formatUptime(75.9)).toBe("1:15");
    });
});

describe("resolveUptimeSecs", () => {
    it("prefers the backend's monotonic uptime when present", () => {
        expect(resolveUptimeSecs(1234, 1_000_000, 999_000)).toBe(1234);
    });

    it("uses the backend value even when the wall clock disagrees wildly", () => {
        // The exact live failure: srv started when the clock read 2081, the
        // clock was corrected back to 2026, so the wall-clock difference is
        // about -1.7e9. The monotonic value is the truth.
        const start = new Date("2081-02-05T08:31:08.693Z").getTime();
        const ts = new Date("2026-08-29T06:32:55.280Z").getTime();
        expect(resolveUptimeSecs(60, ts, start)).toBe(60);
    });

    it("falls back to the wall-clock difference when the backend omits uptime", () => {
        expect(resolveUptimeSecs(undefined, 1_000_000, 940_000)).toBe(60);
        expect(resolveUptimeSecs(null, 1_000_000, 940_000)).toBe(60);
    });

    it("clamps the wall-clock fallback at zero instead of going negative", () => {
        const start = new Date("2081-02-05T08:31:08.693Z").getTime();
        const ts = new Date("2026-08-29T06:32:55.280Z").getTime();
        expect(resolveUptimeSecs(undefined, ts, start)).toBe(0);
    });

    it("rejects a negative or non-finite backend value and falls back", () => {
        expect(resolveUptimeSecs(-5, 1_000_000, 940_000)).toBe(60);
        expect(resolveUptimeSecs(NaN, 1_000_000, 940_000)).toBe(60);
        expect(resolveUptimeSecs("120", 1_000_000, 940_000)).toBe(60);
    });

    it("returns null when neither source is usable, so the caller keeps its last value", () => {
        expect(resolveUptimeSecs(undefined, undefined, 940_000)).toBeNull();
        expect(resolveUptimeSecs(undefined, 1_000_000, null)).toBeNull();
    });

    it("truncates a fractional backend value", () => {
        expect(resolveUptimeSecs(12.9, 1_000_000, 940_000)).toBe(12);
    });
});
