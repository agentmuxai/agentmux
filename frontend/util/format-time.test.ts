// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { formatElapsedClock, formatElapsedCompact } from "./format-time";

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
