// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { compareLastUsed, lastUsedMs, sortByLastUsed, type LastUsedFields } from "./agent-recency";

describe("lastUsedMs", () => {
    it("takes the last activity first, then the launch, then the definition's update", () => {
        expect(lastUsedMs({ last_active_at: 30, started_at: 20, updated_at: 10 })).toBe(30);
        expect(lastUsedMs({ last_active_at: 0, started_at: 20, updated_at: 10 })).toBe(20);
        expect(lastUsedMs({ started_at: null, updated_at: 10 })).toBe(10);
        expect(lastUsedMs({})).toBe(0);
    });
});

describe("sortByLastUsed", () => {
    type Item = LastUsedFields & { name: string };
    const name = (a: Item) => a.name;

    it("puts the most recently used first and never-used last", () => {
        const sorted = sortByLastUsed(
            [
                { name: "old", updated_at: 100 },
                { name: "never" },
                { name: "new", updated_at: 300 },
                { name: "mid", updated_at: 200 },
            ],
            name
        );
        expect(sorted.map(name)).toEqual(["new", "mid", "old", "never"]);
    });

    it("ranks activity over an older launch across row kinds", () => {
        const sorted = sortByLastUsed(
            [
                { name: "launched-late", started_at: 500 },
                { name: "chatted-since", started_at: 100, last_active_at: 900 },
            ],
            name
        );
        expect(sorted.map(name)).toEqual(["chatted-since", "launched-late"]);
    });

    it("keeps name order within a tie and leaves its input alone", () => {
        const input = [
            { name: "b", updated_at: 5 },
            { name: "A", updated_at: 5 },
            { name: "c", updated_at: 5 },
        ];
        expect(sortByLastUsed(input, name).map(name)).toEqual(["A", "b", "c"]);
        expect(input.map(name)).toEqual(["b", "A", "c"]);
    });

    it("compareLastUsed is 0 for equal times", () => {
        expect(compareLastUsed({ updated_at: 1 }, { started_at: 1 })).toBe(0);
    });
});
