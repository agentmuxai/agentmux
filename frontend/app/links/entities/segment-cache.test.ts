// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { buildAgentIndex } from "./indexes";
import { createSegmentCache } from "./segment-cache";
import { linksOf, ROSTER, testContext } from "./test-context";

describe("the segment cache", () => {
    it("returns the same segments for the same message, text and index versions", () => {
        const cache = createSegmentCache();
        const ctx = testContext();
        const a = cache.get("m1", "ask Opaz about #48247", ctx);
        expect(cache.get("m1", "ask Opaz about #48247", ctx)).toBe(a);
        expect(cache.size()).toBe(1);
    });
    it("links again when the text, the roster version or the author changes", () => {
        const cache = createSegmentCache();
        const first = cache.get("m1", "ask Opaz", testContext());
        expect(cache.get("m1", "ask Opaz!", testContext())).not.toBe(first);
        const moved = testContext({ agents: buildAgentIndex([], { version: 2 }) });
        expect(linksOf(cache.get("m1", "ask Opaz", moved))).toEqual([]);
        expect(linksOf(first)).toEqual(["agent:Opaz"]);
        const other = testContext({ author: { kind: "operator" } });
        expect(cache.get("m1", "ask Opaz", other)).not.toBe(first);
    });
    it("keeps no more than its limit", () => {
        const cache = createSegmentCache(3);
        for (let i = 0; i < 10; i++) cache.get(`m${i}`, `text ${i}`, testContext());
        expect(cache.size()).toBe(3);
    });
    it("uses the roster fixture", () => {
        expect(ROSTER.length).toBeGreaterThan(0);
    });
});
