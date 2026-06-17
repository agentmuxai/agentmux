// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { extractRecords, looksLikeRecords, cellText } from "./record-table";

describe("extractRecords", () => {
    it("reads a top-level array of flat records, unioning column order", () => {
        const out = extractRecords([
            { name: "a", count: 1 },
            { name: "b", extra: true },
        ]);
        expect(out?.columns).toEqual(["name", "count", "extra"]);
        expect(out?.rows).toHaveLength(2);
        expect(out?.truncatedRows).toBe(0);
    });

    it("rejects shapes that aren't a flat record list", () => {
        expect(extractRecords([])).toBeNull(); // empty
        expect(extractRecords({ a: 1 })).toBeNull(); // object, not array
        expect(extractRecords("text")).toBeNull(); // string
        expect(extractRecords([1, 2, 3])).toBeNull(); // scalars, not objects
        expect(extractRecords([{ nested: { x: 1 } }])).toBeNull(); // nested object value
        expect(extractRecords([{ arr: [1] }])).toBeNull(); // array value
    });

    it("rejects tables wider than the column cap", () => {
        const wide: Record<string, number> = {};
        for (let i = 0; i < 20; i++) wide[`c${i}`] = i;
        expect(extractRecords([wide])).toBeNull();
    });

    it("caps rows and reports the truncated count", () => {
        const many = Array.from({ length: 250 }, (_, i) => ({ i }));
        const out = extractRecords(many);
        expect(out?.rows).toHaveLength(200);
        expect(out?.truncatedRows).toBe(50);
    });

    it("looksLikeRecords mirrors extract", () => {
        expect(looksLikeRecords([{ a: 1 }])).toBe(true);
        expect(looksLikeRecords({ a: 1 })).toBe(false);
    });

    it("cellText renders scalars and truncates long strings", () => {
        expect(cellText(null)).toBe("");
        expect(cellText(undefined)).toBe("");
        expect(cellText(42)).toBe("42");
        expect(cellText(true)).toBe("true");
        expect(cellText("x".repeat(300))).toHaveLength(201); // 200 + ellipsis
    });
});
