// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { arrivedLive, LIVE_ARRIVAL_WINDOW_MS } from "./live-arrival";

describe("arrivedLive", () => {
    const now = 1_000_000;

    it("is a recent stamp, inside the window", () => {
        expect(arrivedLive(now, now)).toBe(true);
        expect(arrivedLive(now - LIVE_ARRIVAL_WINDOW_MS, now)).toBe(true);
    });

    it("is not an older stamp, or none (a loaded transcript)", () => {
        expect(arrivedLive(now - LIVE_ARRIVAL_WINDOW_MS - 1, now)).toBe(false);
        expect(arrivedLive(undefined, now)).toBe(false);
        expect(arrivedLive(null, now)).toBe(false);
    });
});
