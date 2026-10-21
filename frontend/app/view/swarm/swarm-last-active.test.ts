// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { lastActiveLabel, lastActiveMs, lastActiveTitle } from "./swarm-last-active";

const NOW = 1_700_000_000_000;
const MIN = 60_000;

describe("lastActiveMs", () => {
    it("reads the session's last output time from the block meta", () => {
        expect(lastActiveMs({ "session:last_activity_ms": NOW - 5 * MIN })).toBe(NOW - 5 * MIN);
    });

    it("is 0 for a pane with no stats, a missing meta, or a bad value", () => {
        expect(lastActiveMs(undefined)).toBe(0);
        expect(lastActiveMs(null)).toBe(0);
        expect(lastActiveMs({})).toBe(0);
        expect(lastActiveMs({ "session:last_activity_ms": 0 })).toBe(0);
        expect(lastActiveMs({ "session:last_activity_ms": "yesterday" })).toBe(0);
        expect(lastActiveMs({ "session:last_activity_ms": Number.NaN })).toBe(0);
    });
});

describe("lastActiveLabel", () => {
    it("says just now under a minute, then minutes, hours and days", () => {
        expect(lastActiveLabel(NOW - 20_000, NOW)).toBe("just now");
        expect(lastActiveLabel(NOW - 59_999, NOW)).toBe("just now");
        expect(lastActiveLabel(NOW - 60_000, NOW)).toBe("1m ago");
        expect(lastActiveLabel(NOW - 14 * MIN, NOW)).toBe("14m ago");
        expect(lastActiveLabel(NOW - 125 * MIN, NOW)).toBe("2h ago");
        expect(lastActiveLabel(NOW - 49 * 60 * MIN, NOW)).toBe("2d ago");
    });

    it("never goes negative if the clock steps back", () => {
        expect(lastActiveLabel(NOW + 5 * MIN, NOW)).toBe("just now");
    });

    it("is empty for an agent that has not produced output", () => {
        expect(lastActiveLabel(0, NOW)).toBe("");
        expect(lastActiveTitle(0)).toBe("");
    });
});

describe("lastActiveTitle", () => {
    it("names the exact time", () => {
        expect(lastActiveTitle(NOW)).toMatch(/^Last active \d{1,2}:\d{2}:\d{2} (AM|PM)$/);
    });
});
