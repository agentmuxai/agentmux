// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Paths of `computeComposerRows` that composer-row-layout.test.ts leaves open: a pinned pair's
 * widths in the single-row balance, a lone anchor with no Shell, unsorted slots, and the exact
 * edges of what fits next to what (the gap counts, and a pair that exactly fits is a pair).
 */

import { describe, expect, it } from "vitest";

import { computeComposerRows } from "./composer-row-layout";

const slot = (key: string, width: number) => ({ key, width });
const PAIR = [{ leftKey: "auth", rightKey: "ctx" }];
const row = (left: string[], right: string[]) => ({ left, right });

describe("computeComposerRows — a pinned pair in the single row", () => {
    it("balances the free slots against the pinned widths on the side each member is pinned to (right one wider)", () => {
        const slots = [slot("auth", 10), slot("ctx", 100), slot("a", 50), slot("b", 50)];
        expect(computeComposerRows(slots, "hostShell", 1000, 0, undefined, PAIR)).toEqual([
            row(["auth", "a", "b"], ["ctx"]),
        ]);
    });

    it("balances them the other way when the left member is the wider (and an empty free left is fine then)", () => {
        const slots = [slot("auth", 100), slot("ctx", 10), slot("a", 50), slot("b", 50)];
        expect(computeComposerRows(slots, "hostShell", 1000, 0, undefined, PAIR)).toEqual([
            row(["auth"], ["a", "b", "ctx"]),
        ]);
    });
});

describe("computeComposerRows — the multi-row pairing", () => {
    it("pairs the widest slot with the narrowest whatever order the slots come in", () => {
        const slots = [slot("a", 20), slot("b", 60), slot("c", 40), slot("d", 30)];
        expect(computeComposerRows(slots, "hostShell", 100, 0)).toEqual([row(["b"], ["a"]), row(["c"], ["d"])]);
    });

    it("pairs two slots whose widths and gap exactly fill the line, and splits them when the gap does not fit", () => {
        const slots = [slot("a", 60), slot("b", 50), slot("c", 40), slot("d", 30)];
        expect(computeComposerRows(slots, "hostShell", 95, 5)).toEqual([row(["a"], ["d"]), row(["b"], ["c"])]);
        expect(computeComposerRows(slots, "hostShell", 92, 5)).toEqual([
            row(["a"], []),
            row(["b"], ["d"]),
            row(["c"], []),
        ]);
    });

    it("keeps a lone anchor in the pool when the Shell is not there to pair it with", () => {
        const slots = [slot("runtime", 50), slot("a", 40), slot("b", 30)];
        expect(computeComposerRows(slots, "hostShell", 60, 5, "runtime")).toEqual([
            row(["runtime"], []),
            row(["a"], []),
            row(["b"], []),
        ]);
    });
});

describe("computeComposerRows — a pinned pair's own row", () => {
    const slots = [slot("auth", 40), slot("ctx", 50), slot("x", 100)];

    it("pairs them when their widths and the gap exactly fill the line", () => {
        expect(computeComposerRows(slots, "hostShell", 95, 5, undefined, PAIR)).toEqual([
            row(["x"], []),
            row(["auth"], ["ctx"]),
        ]);
    });

    it("splits them into two rows, each on its pinned side, when the gap does not fit", () => {
        expect(computeComposerRows(slots, "hostShell", 92, 5, undefined, PAIR)).toEqual([
            row(["x"], []),
            row(["auth"], []),
            row([], ["ctx"]),
        ]);
    });
});
