// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, test, expect, beforeEach, afterEach } from "vitest";
import {
    computeInsertIndex,
    computeInsertionPoint,
    computeNearestTab,
    insertionPointToIndex,
    markTabMerged,
    wasTabRecentlyMerged,
    tabWrapperRefs,
    startWindowTabDrag,
    releaseWindowTabDrag,
    endWindowTabDrag,
    draggedWindowTabId,
    decideTabRelease,
    TEAR_PAST_PX,
} from "./tabbar-dnd";
import { beginDrag, endDrag, session } from "@/app/drag/drag-session";

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
        endDrag("cancel");
    });

    afterEach(() => {
        tabWrapperRefs.clear();
        endDrag("cancel");
    });

    test("returns null when no tabs registered", () => {
        expect(computeNearestTab(100, 15)).toBeNull();
    });

    test("returns null when only the dragged tab is registered", () => {
        tabWrapperRefs.set("tab-a", makeFakeEl(0, 100));
        startWindowTabDrag("tab-a", "ws-1", true);
        expect(computeNearestTab(50, 15)).toBeNull();
    });

    test("returns the only non-dragged tab", () => {
        tabWrapperRefs.set("tab-a", makeFakeEl(0, 100));   // mid=50
        tabWrapperRefs.set("tab-b", makeFakeEl(100, 100)); // mid=150
        startWindowTabDrag("tab-a", "ws-1", true);

        const result = computeNearestTab(130, 15);
        expect(result?.tabId).toBe("tab-b");
    });

    test("cursor left of midpoint → side is 'left'", () => {
        tabWrapperRefs.set("tab-a", makeFakeEl(0, 100));   // mid=50
        tabWrapperRefs.set("tab-b", makeFakeEl(100, 100)); // mid=150
        startWindowTabDrag("tab-a", "ws-1", true);

        // cursor at 120 — left of tab-b's midpoint (150)
        const result = computeNearestTab(120, 15);
        expect(result?.tabId).toBe("tab-b");
        expect(result?.side).toBe("left");
    });

    test("cursor right of midpoint → side is 'right'", () => {
        tabWrapperRefs.set("tab-a", makeFakeEl(0, 100));   // mid=50
        tabWrapperRefs.set("tab-b", makeFakeEl(100, 100)); // mid=150
        startWindowTabDrag("tab-a", "ws-1", true);

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
        startWindowTabDrag("tab-b", "ws-1", true); // dragging tab-b

        // cursor at 40 — closer to tab-a (mid=50, dist=10) than tab-c (mid=250, dist=210)
        expect(computeNearestTab(40, 15)?.tabId).toBe("tab-a");

        // cursor at 240 — closer to tab-c (mid=250, dist=10)
        expect(computeNearestTab(240, 15)?.tabId).toBe("tab-c");
    });

    test("cursor exactly at midpoint → side is 'left'", () => {
        tabWrapperRefs.set("tab-a", makeFakeEl(0, 100));   // mid=50
        tabWrapperRefs.set("tab-b", makeFakeEl(100, 100)); // mid=150
        startWindowTabDrag("tab-a", "ws-1", true);

        // exactly at tab-b midpoint (150) → clientX < midX is false → "right"
        // but clientX === midX: 150 < 150 is false → side = "right"
        const result = computeNearestTab(150, 15);
        expect(result?.side).toBe("right");
    });

    test("ignores the dragging tab when computing nearest", () => {
        tabWrapperRefs.set("tab-a", makeFakeEl(0, 100));   // mid=50  ← dragging
        tabWrapperRefs.set("tab-b", makeFakeEl(100, 100)); // mid=150
        tabWrapperRefs.set("tab-c", makeFakeEl(200, 100)); // mid=250
        startWindowTabDrag("tab-a", "ws-1", true);

        // cursor at 60 — closest to tab-a (mid=50) but it's excluded
        // next closest: tab-b (mid=150, dist=90) over tab-c (mid=250, dist=190)
        expect(computeNearestTab(60, 15)?.tabId).toBe("tab-b");
    });
});

// ── computeInsertionPoint ─────────────────────────────────────────────────────
//
// Production reorder rule (replaced computeNearestTab in tabbar.tsx). Thresholds
// on each remaining tab's CENTER, not the inter-tab gap: the dragged tab stays
// in the strip at opacity 0.35 but is EXCLUDED from the scan, so a gap-midpoint
// rule measured the cursor against the empty space it still occupied and landed
// the "cross to move" threshold ~1.5–2 tab-widths away. makeFakeEl(left, width)
// ⇒ a tab spanning [left, left+width], so center = left + width/2.

describe("computeInsertionPoint", () => {
    beforeEach(() => {
        tabWrapperRefs.clear();
        endDrag("cancel");
    });
    afterEach(() => {
        tabWrapperRefs.clear();
        endDrag("cancel");
    });

    test("returns null when no tabs are registered", () => {
        expect(computeInsertionPoint(100)).toBeNull();
    });

    test("returns null when only the dragged tab is registered", () => {
        tabWrapperRefs.set("tab-a", makeFakeEl(0, 100));
        startWindowTabDrag("tab-a", "ws-1", true);
        expect(computeInsertionPoint(50)).toBeNull();
    });

    test("cursor left of the first tab's center → insert before the first tab", () => {
        tabWrapperRefs.set("tab-a", makeFakeEl(0, 100));   // center 50
        tabWrapperRefs.set("tab-b", makeFakeEl(100, 100)); // center 150
        // 40 < 50 → before tab-a
        expect(computeInsertionPoint(40)).toEqual({ beforeTabId: null, afterTabId: "tab-a" });
    });

    test("cursor between two centers → lands in that gap", () => {
        tabWrapperRefs.set("tab-a", makeFakeEl(0, 100));   // center 50
        tabWrapperRefs.set("tab-b", makeFakeEl(100, 100)); // center 150
        tabWrapperRefs.set("tab-c", makeFakeEl(200, 100)); // center 250
        // 160: past 50 and 150, < 250 → between tab-b and tab-c
        expect(computeInsertionPoint(160)).toEqual({ beforeTabId: "tab-b", afterTabId: "tab-c" });
    });

    test("cursor past every center → append after the last tab", () => {
        tabWrapperRefs.set("tab-a", makeFakeEl(0, 100));   // center 50
        tabWrapperRefs.set("tab-b", makeFakeEl(100, 100)); // center 150
        expect(computeInsertionPoint(500)).toEqual({ beforeTabId: "tab-b", afterTabId: null });
    });

    test("exactly at a center goes to the next gap (clientX < center is exclusive)", () => {
        tabWrapperRefs.set("tab-a", makeFakeEl(0, 100));   // center 50
        tabWrapperRefs.set("tab-b", makeFakeEl(100, 100)); // center 150
        // 50 < 50 is false → move on; 50 < 150 → between tab-a and tab-b
        expect(computeInsertionPoint(50)).toEqual({ beforeTabId: "tab-a", afterTabId: "tab-b" });
    });

    test("crossing a neighbour's CENTER commits the move (regression: not the gap midpoint)", () => {
        // Drag tab-a (excluded from the scan). Remaining: tab-b(center150),
        // tab-c(center250). The move to after tab-b must commit as soon as the
        // cursor crosses tab-b's center (150) — NOT the tab-b/tab-c gap
        // midpoint (200), which was the old "too tight" behaviour.
        tabWrapperRefs.set("tab-a", makeFakeEl(0, 100));   // dragged, excluded
        tabWrapperRefs.set("tab-b", makeFakeEl(100, 100)); // center 150
        tabWrapperRefs.set("tab-c", makeFakeEl(200, 100)); // center 250
        startWindowTabDrag("tab-a", "ws-1", true);
        // just left of tab-b's center → before tab-b
        expect(computeInsertionPoint(149)).toEqual({ beforeTabId: null, afterTabId: "tab-b" });
        // just right of tab-b's center → after tab-b (well short of the gap midpoint)
        expect(computeInsertionPoint(151)).toEqual({ beforeTabId: "tab-b", afterTabId: "tab-c" });
    });

    test("scan order is left-to-right regardless of insertion order into the registry", () => {
        // Registry insertion order is reversed; computeInsertionPoint sorts by left.
        tabWrapperRefs.set("tab-c", makeFakeEl(200, 100)); // center 250
        tabWrapperRefs.set("tab-a", makeFakeEl(0, 100));   // center 50
        tabWrapperRefs.set("tab-b", makeFakeEl(100, 100)); // center 150
        expect(computeInsertionPoint(40)).toEqual({ beforeTabId: null, afterTabId: "tab-a" });
        expect(computeInsertionPoint(300)).toEqual({ beforeTabId: "tab-c", afterTabId: null });
    });
});

// ── insertionPointToIndex ─────────────────────────────────────────────────────
//
// Cross-window merge/remount index conversion: the incoming tab is NOT in
// `tabs`, so no removal-shift adjustment (SPEC_CROSS_WINDOW_TAB_REMOUNT §4.2).

describe("insertionPointToIndex", () => {
    const tabs = ["a", "b", "c"];

    test("null insertion point appends at end", () => {
        expect(insertionPointToIndex(null, tabs)).toBe(3);
        expect(insertionPointToIndex(null, [])).toBe(0);
    });

    test("before the first tab inserts at 0", () => {
        expect(insertionPointToIndex({ beforeTabId: null, afterTabId: "a" }, tabs)).toBe(0);
    });

    test("after the last tab appends at end", () => {
        expect(insertionPointToIndex({ beforeTabId: "c", afterTabId: null }, tabs)).toBe(3);
    });

    test("between two tabs inserts at the afterTabId position", () => {
        expect(insertionPointToIndex({ beforeTabId: "a", afterTabId: "b" }, tabs)).toBe(1);
        expect(insertionPointToIndex({ beforeTabId: "b", afterTabId: "c" }, tabs)).toBe(2);
    });

    test("unknown afterTabId falls back to append", () => {
        expect(insertionPointToIndex({ beforeTabId: "a", afterTabId: "ghost" }, tabs)).toBe(3);
    });
});

// ── cross-window merge dedup ──────────────────────────────────────────────────

describe("markTabMerged / wasTabRecentlyMerged", () => {
    test("unmarked tab is not recently merged", () => {
        expect(wasTabRecentlyMerged("never-marked")).toBe(false);
    });

    test("marked tab is recently merged within the window", () => {
        markTabMerged("t1", 1_000_000);
        expect(wasTabRecentlyMerged("t1", 1_000_100)).toBe(true);
        expect(wasTabRecentlyMerged("t1", 1_004_999)).toBe(true);
    });

    test("mark expires after the dedup window and is pruned", () => {
        markTabMerged("t2", 1_000_000);
        expect(wasTabRecentlyMerged("t2", 1_006_000)).toBe(false);
        // Pruned on the expired read — a later in-window read stays false.
        expect(wasTabRecentlyMerged("t2", 1_000_100)).toBe(false);
    });
});

// ── decideTabRelease ──────────────────────────────────────────────────────────
//
// Strip: x 0-500, y 0-30. Tear-off needs a release more than TEAR_PAST_PX
// below it.

describe("decideTabRelease", () => {
    const strip = { left: 0, right: 500, top: 0, bottom: 30 };
    const ip = { beforeTabId: "tab-a", afterTabId: "tab-b" };
    const base = { escaped: false, ip, stripRect: strip, tabCount: 3, draggedTabId: "tab-c", canTearOff: true };
    const at = (clientX: number, clientY: number) => ({ clientX, clientY });

    test("a release inside the strip with an insertion point reorders", () => {
        expect(decideTabRelease({ ...base, input: at(200, 15) })).toBe("reorder");
    });

    test("inside the strip without an insertion point does nothing", () => {
        expect(decideTabRelease({ ...base, ip: null, input: at(200, 15) })).toBe("none");
    });

    test("a release below the strip tears off, even with an insertion point", () => {
        expect(decideTabRelease({ ...base, input: at(200, 30 + TEAR_PAST_PX + 1) })).toBe("tear-off");
    });

    test("within TEAR_PAST_PX below the strip does nothing", () => {
        expect(decideTabRelease({ ...base, input: at(200, 30 + TEAR_PAST_PX) })).toBe("none");
    });

    test("beside or above the strip does nothing", () => {
        expect(decideTabRelease({ ...base, input: at(600, 15) })).toBe("none");
        expect(decideTabRelease({ ...base, input: at(200, -20) })).toBe("none");
    });

    test("a lone tab never tears off", () => {
        expect(decideTabRelease({ ...base, tabCount: 1, input: at(200, 200) })).toBe("none");
    });

    test("Escape aborts, wherever the release is", () => {
        expect(decideTabRelease({ ...base, escaped: true, input: at(200, 15) })).toBe("abort");
        expect(decideTabRelease({ ...base, escaped: true, input: at(200, 200) })).toBe("abort");
    });

    test("a host that can't tear off does nothing below the strip", () => {
        expect(decideTabRelease({ ...base, canTearOff: false, input: at(200, 200) })).toBe("none");
        expect(decideTabRelease({ ...base, canTearOff: false, input: at(200, 15) })).toBe("reorder");
    });

    test("with no strip rect nothing happens", () => {
        expect(decideTabRelease({ ...base, stripRect: null, input: at(200, 200) })).toBe("none");
    });
});

// ── the window-tab drag session ──────────────────────────────────────────────

describe("window-tab drag session", () => {
    afterEach(() => endDrag("cancel"));

    test("start begins a window-tab session with its tab, workspace and eligibility", () => {
        startWindowTabDrag("tab-a", "ws-1", true);
        expect(session()).toMatchObject({
            kind: "window-tab",
            source: { tabId: "tab-a", wsId: "ws-1" },
            payload: { crossWindow: true },
            released: false,
        });
        expect(draggedWindowTabId()).toBe("tab-a");
    });

    test("a lone-tab drag is not cross-window eligible", () => {
        startWindowTabDrag("tab-a", "ws-1", false);
        expect(session()?.payload?.crossWindow).toBe(false);
    });

    test("the source's release keeps the session but no tab counts as dragged", () => {
        startWindowTabDrag("tab-a", "ws-1", true);
        releaseWindowTabDrag();
        expect(session()?.released).toBe(true);
        expect(draggedWindowTabId()).toBeNull();
    });

    test("the tab bar's monitor ends it", () => {
        startWindowTabDrag("tab-a", "ws-1", true);
        releaseWindowTabDrag();
        endWindowTabDrag("drop");
        expect(session()).toBeNull();
    });

    test("never releases or ends another kind of drag", () => {
        beginDrag("tile", { nodeId: "n1" });
        releaseWindowTabDrag();
        endWindowTabDrag("drop");
        expect(session()).toMatchObject({ kind: "tile", released: false });
        expect(draggedWindowTabId()).toBeNull();
    });

    test("a stranded window-tab drag stops counting once another drag begins", () => {
        startWindowTabDrag("tab-a", "ws-1", true);
        beginDrag("tile", { nodeId: "n1" });
        expect(draggedWindowTabId()).toBeNull();
    });
});
