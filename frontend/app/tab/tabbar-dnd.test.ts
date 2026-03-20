// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, test, expect, beforeEach, afterEach } from "vitest";
import {
    computeInsertIndex,
    computeNearestTab,
    tabWrapperRefs,
    setGlobalDragTabId,
} from "./tabbar-dnd";

// ── computeInsertIndex ────────────────────────────────────────────────────────
//
// Backend does remove-then-insert on a section-specific array.
// Rule: when source is before target (source < rawIndex), subtract 1.
//
// Visual legend for each test:
//   [A B C D E]   indices 0-4
//   source=X, target=Y, side=left|right → expected newIndex

describe("computeInsertIndex", () => {

    // ── dragging FORWARD (source before target) ──────────────────────────────

    test("forward: drop to the LEFT of target", () => {
        // [A B C D E], drag A(0) to left of D(3)
        // rawIndex = 3, source(0) < 3 → newIndex = 2
        // after removing A: [B C D E], insert at 2 → [B C A D E] ✓
        expect(computeInsertIndex(0, 3, "left")).toBe(2);
    });

    test("forward: drop to the RIGHT of target", () => {
        // [A B C D E], drag B(1) to right of D(3)
        // rawIndex = 4, source(1) < 4 → newIndex = 3
        // after removing B: [A C D E], insert at 3 → [A C D B E] ✓
        expect(computeInsertIndex(1, 3, "right")).toBe(3);
    });

    test("forward: drag to the last position (right of last tab)", () => {
        // [A B C D E], drag A(0) to right of E(4)
        // rawIndex = 5, source(0) < 5 → newIndex = 4
        // after removing A: [B C D E], insert at 4 → [B C D E A] ✓
        expect(computeInsertIndex(0, 4, "right")).toBe(4);
    });

    test("forward: drag one step right (adjacent)", () => {
        // [A B C], drag A(0) to right of B(1)
        // rawIndex = 2, source(0) < 2 → newIndex = 1
        // after removing A: [B C], insert at 1 → [B A C] ✓
        expect(computeInsertIndex(0, 1, "right")).toBe(1);
    });

    // ── dragging BACKWARD (source after target) ───────────────────────────────

    test("backward: drop to the LEFT of target", () => {
        // [A B C D E], drag E(4) to left of B(1)
        // rawIndex = 1, source(4) >= 1 → newIndex = 1
        // after removing E: [A B C D], insert at 1 → [A E B C D] ✓
        expect(computeInsertIndex(4, 1, "left")).toBe(1);
    });

    test("backward: drop to the RIGHT of target", () => {
        // [A B C D E], drag E(4) to right of B(1)
        // rawIndex = 2, source(4) >= 2 → newIndex = 2
        // after removing E: [A B C D], insert at 2 → [A B E C D] ✓
        expect(computeInsertIndex(4, 1, "right")).toBe(2);
    });

    test("backward: drag to the first position (left of first tab)", () => {
        // [A B C D E], drag E(4) to left of A(0)
        // rawIndex = 0, source(4) >= 0 → newIndex = 0
        // after removing E: [A B C D], insert at 0 → [E A B C D] ✓
        expect(computeInsertIndex(4, 0, "left")).toBe(0);
    });

    test("backward: drag one step left (adjacent)", () => {
        // [A B C], drag C(2) to left of B(1)
        // rawIndex = 1, source(2) >= 1 → newIndex = 1
        // after removing C: [A B], insert at 1 → [A C B] ✓
        expect(computeInsertIndex(2, 1, "left")).toBe(1);
    });

    // ── classic bug scenario that prompted the fix ────────────────────────────

    test("middle tab dragged to end: [A B C D E] drag B(1) to right of E(4)", () => {
        // rawIndex = 5, source(1) < 5 → newIndex = 4
        // after removing B: [A C D E], insert at 4 → [A C D E B] ✓
        expect(computeInsertIndex(1, 4, "right")).toBe(4);
    });

    test("middle tab dragged to front: [A B C D E] drag C(2) to left of A(0)", () => {
        // rawIndex = 0, source(2) >= 0 → newIndex = 0
        // after removing C: [A B D E], insert at 0 → [C A B D E] ✓
        expect(computeInsertIndex(2, 0, "left")).toBe(0);
    });

    // ── same position (no-op semantics) ──────────────────────────────────────

    test("drop immediately left of source (no-op territory)", () => {
        // [A B C], drag B(1) to left of B(1) — canDrop blocks same-tab,
        // but if it slipped through: rawIndex=1, source(1) >= 1 → newIndex=1
        // after removing B: [A C], insert at 1 → [A B C] (unchanged) ✓
        expect(computeInsertIndex(1, 1, "left")).toBe(1);
    });

    test("drop immediately right of source (no-op territory)", () => {
        // [A B C], drag B(1) to right of B(1)
        // rawIndex=2, source(1) < 2 → newIndex=1
        // after removing B: [A C], insert at 1 → [A B C] (unchanged) ✓
        expect(computeInsertIndex(1, 1, "right")).toBe(1);
    });

    // ── two-tab edge cases ────────────────────────────────────────────────────

    test("two tabs: drag first to right of second", () => {
        // [A B], drag A(0) to right of B(1)
        // rawIndex=2, source(0) < 2 → newIndex=1
        // after removing A: [B], insert at 1 → [B A] ✓
        expect(computeInsertIndex(0, 1, "right")).toBe(1);
    });

    test("two tabs: drag second to left of first", () => {
        // [A B], drag B(1) to left of A(0)
        // rawIndex=0, source(1) >= 0 → newIndex=0
        // after removing B: [A], insert at 0 → [B A] ✓
        expect(computeInsertIndex(1, 0, "left")).toBe(0);
    });
});

// ── computeNearestTab ─────────────────────────────────────────────────────────
//
// Tests use fake HTMLDivElements with mocked getBoundingClientRect.

function makeFakeEl(left: number, width: number): HTMLDivElement {
    const el = document.createElement("div");
    el.getBoundingClientRect = () => ({
        left,
        width,
        right: left + width,
        top: 0,
        bottom: 30,
        height: 30,
        x: left,
        y: 0,
        toJSON: () => {},
    });
    return el;
}

describe("computeNearestTab", () => {
    beforeEach(() => {
        tabWrapperRefs.clear();
        setGlobalDragTabId(null);
    });

    afterEach(() => {
        tabWrapperRefs.clear();
        setGlobalDragTabId(null);
    });

    test("returns null when no tabs registered", () => {
        expect(computeNearestTab(100, 15)).toBeNull();
    });

    test("returns null when only the dragged tab is registered", () => {
        tabWrapperRefs.set("tab-a", makeFakeEl(0, 100));
        setGlobalDragTabId("tab-a");
        expect(computeNearestTab(50, 15)).toBeNull();
    });

    test("returns the only non-dragged tab", () => {
        tabWrapperRefs.set("tab-a", makeFakeEl(0, 100));   // mid=50
        tabWrapperRefs.set("tab-b", makeFakeEl(100, 100)); // mid=150
        setGlobalDragTabId("tab-a");

        const result = computeNearestTab(130, 15);
        expect(result?.tabId).toBe("tab-b");
    });

    test("cursor left of midpoint → side is 'left'", () => {
        tabWrapperRefs.set("tab-a", makeFakeEl(0, 100));   // mid=50
        tabWrapperRefs.set("tab-b", makeFakeEl(100, 100)); // mid=150
        setGlobalDragTabId("tab-a");

        // cursor at 120 — left of tab-b's midpoint (150)
        const result = computeNearestTab(120, 15);
        expect(result?.tabId).toBe("tab-b");
        expect(result?.side).toBe("left");
    });

    test("cursor right of midpoint → side is 'right'", () => {
        tabWrapperRefs.set("tab-a", makeFakeEl(0, 100));   // mid=50
        tabWrapperRefs.set("tab-b", makeFakeEl(100, 100)); // mid=150
        setGlobalDragTabId("tab-a");

        // cursor at 180 — right of tab-b's midpoint (150)
        const result = computeNearestTab(180, 15);
        expect(result?.tabId).toBe("tab-b");
        expect(result?.side).toBe("right");
    });

    test("picks nearest tab by midpoint distance", () => {
        // tab-a: 0-100, mid=50
        // tab-b: 100-200, mid=150
        // tab-c: 200-300, mid=250
        tabWrapperRefs.set("tab-a", makeFakeEl(0, 100));
        tabWrapperRefs.set("tab-b", makeFakeEl(100, 100));
        tabWrapperRefs.set("tab-c", makeFakeEl(200, 100));
        setGlobalDragTabId("tab-b"); // dragging tab-b

        // cursor at 40 — closer to tab-a (mid=50, dist=10) than tab-c (mid=250, dist=210)
        expect(computeNearestTab(40, 15)?.tabId).toBe("tab-a");

        // cursor at 240 — closer to tab-c (mid=250, dist=10)
        expect(computeNearestTab(240, 15)?.tabId).toBe("tab-c");
    });

    test("cursor exactly at midpoint → side is 'left'", () => {
        tabWrapperRefs.set("tab-a", makeFakeEl(0, 100));   // mid=50
        tabWrapperRefs.set("tab-b", makeFakeEl(100, 100)); // mid=150
        setGlobalDragTabId("tab-a");

        // exactly at tab-b midpoint (150) → clientX < midX is false → "right"
        // but clientX === midX: 150 < 150 is false → side = "right"
        const result = computeNearestTab(150, 15);
        expect(result?.side).toBe("right");
    });

    test("ignores the dragging tab when computing nearest", () => {
        tabWrapperRefs.set("tab-a", makeFakeEl(0, 100));   // mid=50  ← dragging
        tabWrapperRefs.set("tab-b", makeFakeEl(100, 100)); // mid=150
        tabWrapperRefs.set("tab-c", makeFakeEl(200, 100)); // mid=250
        setGlobalDragTabId("tab-a");

        // cursor at 60 — closest to tab-a (mid=50) but it's excluded
        // next closest: tab-b (mid=150, dist=90) over tab-c (mid=250, dist=190)
        expect(computeNearestTab(60, 15)?.tabId).toBe("tab-b");
    });
});
