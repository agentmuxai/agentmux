// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { growthLabel, growthOver, sparklinePoints, versionDelta } from "./memory-history-tokens";

const DAY = 86_400_000;
const NOW = 100 * DAY;
// Newest first, as the history lists them.
const versions = [
    { created_at: NOW - 1 * DAY, tokens_est: 1500 },
    { created_at: NOW - 5 * DAY, tokens_est: 1200 },
    { created_at: NOW - 10 * DAY, tokens_est: 1240 },
    { created_at: NOW - 40 * DAY, tokens_est: 300 },
];

describe("memory-history-tokens", () => {
    it("gives each version's change from the one before it", () => {
        expect(versionDelta(versions, 0)).toBe("+300");
        expect(versionDelta(versions, 1)).toBe("−40");
        expect(versionDelta(versions, 2)).toBe("+940");
        expect(versionDelta(versions, 3)).toBe("", );
        expect(versionDelta([{ created_at: 1, tokens_est: 5 }, { created_at: 0, tokens_est: 5 }], 0)).toBe("");
    });

    it("measures growth from the size current at the cutoff, or from nothing for a younger file", () => {
        expect(growthOver(versions, 7, NOW)).toBe(1500 - 1240);
        expect(growthOver(versions, 30, NOW)).toBe(1500 - 300);
        expect(growthOver(versions, 60, NOW)).toBe(1500);
        expect(growthOver([], 7, NOW)).toBeNull();
        expect(growthOver([{ created_at: NOW }], 7, NOW)).toBeNull();
    });

    it("labels growth in tokens, or says there was none", () => {
        expect(growthLabel(1200)).toBe("+1.2k tok");
        expect(growthLabel(-40)).toBe("−40 tok");
        expect(growthLabel(0)).toBe("no change");
    });

    it("draws the sizes oldest first, highest at the top, and nothing for a single version", () => {
        expect(sparklinePoints(versions, 120, 18)).toBe("0.0,14.4 40.0,3.1 80.0,3.6 120.0,0.0");
        expect(sparklinePoints(versions.slice(0, 1), 120, 18)).toBe("");
    });
});
