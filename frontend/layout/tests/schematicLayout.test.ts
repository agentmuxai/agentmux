// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, test } from "vitest";
import { computeSchematicRects, schematicLeaves } from "../lib/schematicLayout";
import { newLayoutNode } from "../lib/layoutNode";
import { FlexDirection, LayoutNode } from "../lib/types";

// Same flex-pool math as layoutGeometry.ts's updateTreeHelper, but as a
// standalone pure function — see SPEC_PANE_DRAG_TO_TAB_2026_07_10.md §4.3.
// These tests exercise the math against a bare Dimensions rect, without any
// LayoutModel/DOM involved, which is exactly the point of it being pure.

describe("computeSchematicRects", () => {
    test("single leaf root gets the full bounding rect", () => {
        const root = newLayoutNode(FlexDirection.Row, undefined, undefined, { blockId: "b1" });
        const rects = computeSchematicRects(root, { top: 0, left: 0, width: 200, height: 100 });
        expect(rects.get(root.id)).toEqual({ top: 0, left: 0, width: 200, height: 100 });
    });

    test("row split divides width proportionally to each child's size", () => {
        const leafA = newLayoutNode(FlexDirection.Row, undefined, undefined, { blockId: "a" });
        const leafB = newLayoutNode(FlexDirection.Row, undefined, undefined, { blockId: "b" });
        leafA.size = 1;
        leafB.size = 3;
        const root: LayoutNode = {
            id: "root",
            flexDirection: FlexDirection.Row,
            size: 1,
            children: [leafA, leafB],
        };
        const rects = computeSchematicRects(root, { top: 0, left: 0, width: 200, height: 100 });
        // total size 4 over 200px → 50px per unit. leafA gets 1 unit (50px), leafB gets 3 (150px).
        expect(rects.get(leafA.id)).toEqual({ top: 0, left: 0, width: 50, height: 100 });
        expect(rects.get(leafB.id)).toEqual({ top: 0, left: 50, width: 150, height: 100 });
    });

    test("column split divides height proportionally, full width per child", () => {
        const leafA = newLayoutNode(FlexDirection.Column, undefined, undefined, { blockId: "a" });
        const leafB = newLayoutNode(FlexDirection.Column, undefined, undefined, { blockId: "b" });
        leafA.size = 1;
        leafB.size = 1;
        const root: LayoutNode = {
            id: "root",
            flexDirection: FlexDirection.Column,
            size: 1,
            children: [leafA, leafB],
        };
        const rects = computeSchematicRects(root, { top: 0, left: 0, width: 200, height: 100 });
        expect(rects.get(leafA.id)).toEqual({ top: 0, left: 0, width: 200, height: 50 });
        expect(rects.get(leafB.id)).toEqual({ top: 50, left: 0, width: 200, height: 50 });
    });

    test("nested split: a column inside one half of a row", () => {
        const leafTop = newLayoutNode(FlexDirection.Column, undefined, undefined, { blockId: "top" });
        const leafBottom = newLayoutNode(FlexDirection.Column, undefined, undefined, { blockId: "bottom" });
        leafTop.size = 1;
        leafBottom.size = 1;
        const rightColumn: LayoutNode = {
            id: "rightColumn",
            flexDirection: FlexDirection.Column,
            size: 1,
            children: [leafTop, leafBottom],
        };
        const leafLeft = newLayoutNode(FlexDirection.Row, undefined, undefined, { blockId: "left" });
        leafLeft.size = 1;
        const root: LayoutNode = {
            id: "root",
            flexDirection: FlexDirection.Row,
            size: 1,
            children: [leafLeft, rightColumn],
        };
        const rects = computeSchematicRects(root, { top: 0, left: 0, width: 200, height: 100 });
        expect(rects.get(leafLeft.id)).toEqual({ top: 0, left: 0, width: 100, height: 100 });
        expect(rects.get(rightColumn.id)).toEqual({ top: 0, left: 100, width: 100, height: 100 });
        expect(rects.get(leafTop.id)).toEqual({ top: 0, left: 100, width: 100, height: 50 });
        expect(rects.get(leafBottom.id)).toEqual({ top: 50, left: 100, width: 100, height: 50 });
    });

    test("zero-size bounding rect does not throw or produce NaN rects", () => {
        const leafA = newLayoutNode(FlexDirection.Row, undefined, undefined, { blockId: "a" });
        const leafB = newLayoutNode(FlexDirection.Row, undefined, undefined, { blockId: "b" });
        leafA.size = 1;
        leafB.size = 1;
        const root: LayoutNode = { id: "root", flexDirection: FlexDirection.Row, size: 1, children: [leafA, leafB] };
        expect(() => computeSchematicRects(root, { top: 0, left: 0, width: 0, height: 0 })).not.toThrow();
        const rects = computeSchematicRects(root, { top: 0, left: 0, width: 0, height: 0 });
        expect(rects.has(leafA.id)).toBe(false);
        expect(rects.has(leafB.id)).toBe(false);
    });
});

describe("schematicLeaves", () => {
    test("returns only leaves with a blockId, skipping branch nodes", () => {
        const leafA = newLayoutNode(FlexDirection.Row, undefined, undefined, { blockId: "a" });
        const leafB = newLayoutNode(FlexDirection.Row, undefined, undefined, { blockId: "b" });
        leafA.size = 1;
        leafB.size = 1;
        const root: LayoutNode = { id: "root", flexDirection: FlexDirection.Row, size: 1, children: [leafA, leafB] };
        const rects = computeSchematicRects(root, { top: 0, left: 0, width: 200, height: 100 });
        const leaves = schematicLeaves(root, rects);
        expect(leaves).toHaveLength(2);
        expect(leaves.map((l) => l.blockId).sort()).toEqual(["a", "b"]);
        expect(leaves.find((l) => l.blockId === "a")?.rect).toEqual({ top: 0, left: 0, width: 100, height: 100 });
    });

    test("skips a leaf with no computed rect (e.g. degenerate zero-size tree)", () => {
        const leafA = newLayoutNode(FlexDirection.Row, undefined, undefined, { blockId: "a" });
        const leafB = newLayoutNode(FlexDirection.Row, undefined, undefined, { blockId: "b" });
        leafA.size = 1;
        leafB.size = 1;
        const root: LayoutNode = { id: "root", flexDirection: FlexDirection.Row, size: 1, children: [leafA, leafB] };
        const rects = computeSchematicRects(root, { top: 0, left: 0, width: 0, height: 0 });
        expect(schematicLeaves(root, rects)).toEqual([]);
    });
});
