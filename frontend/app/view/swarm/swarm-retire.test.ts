// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { countdownCandidates, withRetiredEntry } from "./swarm-retire";

describe("retiring a row again", () => {
    it("keeps the same Map when the row is already retired at that event time, so nothing rebuilds", () => {
        const retired = new Map([["agent:a", 100]]);
        expect(withRetiredEntry(retired, "agent:a", 100)).toBe(retired);
    });

    it("makes a new Map for a new row or newer activity", () => {
        const retired = new Map([["agent:a", 100]]);
        const added = withRetiredEntry(retired, "agent:b", 5);
        expect(added).not.toBe(retired);
        expect([...added]).toEqual([["agent:a", 100], ["agent:b", 5]]);
        expect(withRetiredEntry(retired, "agent:a", 200).get("agent:a")).toBe(200);
        expect(retired.get("agent:a")).toBe(100);
    });
});

describe("which rows get an auto-linger countdown", () => {
    it("leaves out a row already retired at its current event time", () => {
        const live = new Map([["agent:a", 100], ["agent:b", 50]]);
        const retired = new Map([["agent:a", 100]]);
        expect([...countdownCandidates(live, retired)]).toEqual([["agent:b", 50]]);
    });

    it("counts a retired row again once it has new activity", () => {
        const live = new Map([["agent:a", 150]]);
        const retired = new Map([["agent:a", 100]]);
        expect([...countdownCandidates(live, retired)]).toEqual([["agent:a", 150]]);
    });
});
