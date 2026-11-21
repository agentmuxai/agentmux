// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { LayoutView, RowPosition } from "../../../store/agent-pane-layout/reducer";
import { SKELETON_MAX_ROWS, skeletonRows } from "./skeleton-band";

const rowsOf = (n: number, height = 100): RowPosition[] =>
    Array.from({ length: n }, (_, i) => ({ nodeId: `n${i}`, index: i, start: i * (height + 4), height, end: i * (height + 4) + height }));

const view = (rows: RowPosition[], startIndex: number, endIndex: number): LayoutView =>
    ({ rows, totalSize: 0, window: { startIndex, endIndex }, scrollMarginPx: 0 }) as unknown as LayoutView;

describe("skeletonRows", () => {
    it("is the rows within the band either side of the window, and none of the window's own", () => {
        const rows = rowsOf(100);
        const got = skeletonRows(view(rows, 40, 45), 520).map((r) => r.index);
        // 520 px is five rows of 104 px past each edge.
        expect(got.sort((a, b) => a - b)).toEqual([35, 36, 37, 38, 39, 46, 47, 48, 49, 50]);
    });

    it("stops at the ends of the document", () => {
        const rows = rowsOf(10);
        const got = skeletonRows(view(rows, 0, 3), 10_000).map((r) => r.index);
        expect(got.sort((a, b) => a - b)).toEqual([4, 5, 6, 7, 8, 9]);
    });

    it("is empty with no layout, an empty window, or no rows", () => {
        expect(skeletonRows(undefined)).toEqual([]);
        expect(skeletonRows(view(rowsOf(5), 3, 2))).toEqual([]);
        expect(skeletonRows(view([], 0, -1))).toEqual([]);
    });

    it("never draws more than the cap, however small the rows", () => {
        const rows = rowsOf(2000, 4);
        expect(skeletonRows(view(rows, 1000, 1010), 100_000).length).toBeLessThanOrEqual(SKELETON_MAX_ROWS);
    });
});
