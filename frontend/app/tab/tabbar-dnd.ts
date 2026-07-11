// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal } from "solid-js";

export const tabItemType = "TAB_ITEM";

/** Half the gap opened on each side of an insertion point (px). Total visual gap = 2 × GAP_PX. */
export const GAP_PX = 12;

// ── Shared drag state ──────────────────────────────────────────────────────

export let globalDragTabId: string | null = null;
export function setGlobalDragTabId(id: string | null): void {
    globalDragTabId = id;
}

// ── Insertion point ────────────────────────────────────────────────────────
// The gap between two tabs where the dragged tab will land.
// null  beforeTabId → gap is before the very first tab
// null  afterTabId  → gap is after the very last tab

export type InsertionPoint = {
    beforeTabId: string | null;
    afterTabId: string | null;
};

export const [insertionPoint, setInsertionPoint] = createSignal<InsertionPoint | null>(null);

// Which tab (by id) should play the landing bounce animation.
export const [bouncingTabId, setBouncingTabId] = createSignal<string | null>(null);

// Which tab (by id), if any, a pane (tile) drag is currently hovering in
// the tab bar. Drives the `.tile-drop-hover` pulse (see tabbar.scss) — the
// "tab blinks" beat of SPEC_PANE_DRAG_TO_TAB_2026_07_10.md, shown during
// the spring-switch dwell. Set/cleared by DroppableTab's tile drop target.
export const [hoveredDropTabId, setHoveredDropTabId] = createSignal<string | null>(null);

// How long a pane drag must dwell over a tab before the UI spring-switches
// to it (the blink plays during this window). Modeled on browser/VS Code
// spring-loading; deliberately longer than REDOCK_DWELL_MS's 180ms ghost
// gate — switching the whole visible tab is a bigger action than showing a
// ghost, so it gets a more deliberate threshold.
export const SPRING_SWITCH_MS = 500;

// Tabs whose LayoutModel.activeDrag was force-set by a mid-drag spring
// switch (DroppableTab) so their TileLayout overlay accepts the foreign
// pane. TileLayout's own drag-end cleanup only resets the SOURCE tab's
// model — tabbar.tsx's tile monitor resets these at end of drag.
export const dragActivatedTabIds = new Set<string>();

// Registry of tab wrapper elements, keyed by tabId.
export const tabWrapperRefs = new Map<string, HTMLDivElement>();

// ── Utilities ──────────────────────────────────────────────────────────────

/**
 * Returns the insertion point (gap) closest to clientX.
 * Gaps considered: before first tab, between each pair, after last tab.
 * The dragged tab is excluded from the registry scan.
 */
export function computeInsertionPoint(clientX: number): InsertionPoint | null {
    const tabs: { tabId: string; left: number; right: number }[] = [];
    for (const [tabId, el] of tabWrapperRefs) {
        if (tabId === globalDragTabId) continue;
        const rect = el.getBoundingClientRect();
        tabs.push({ tabId, left: rect.left, right: rect.right });
    }
    if (tabs.length === 0) return null;
    tabs.sort((a, b) => a.left - b.left);

    let bestDist = Infinity;
    let result: InsertionPoint = { beforeTabId: null, afterTabId: tabs[0].tabId };

    // Gap before first tab
    const d0 = Math.abs(clientX - tabs[0].left);
    if (d0 < bestDist) {
        bestDist = d0;
        result = { beforeTabId: null, afterTabId: tabs[0].tabId };
    }

    // Gaps between adjacent tabs
    for (let i = 0; i < tabs.length - 1; i++) {
        const gapX = (tabs[i].right + tabs[i + 1].left) / 2;
        const dist = Math.abs(clientX - gapX);
        if (dist < bestDist) {
            bestDist = dist;
            result = { beforeTabId: tabs[i].tabId, afterTabId: tabs[i + 1].tabId };
        }
    }

    // Gap after last tab
    const last = tabs[tabs.length - 1];
    const dLast = Math.abs(clientX - last.right);
    if (dLast < bestDist) {
        result = { beforeTabId: last.tabId, afterTabId: null };
    }

    return result;
}

/**
 * Kept for unit tests. Production code uses computeInsertionPoint.
 */
export function computeNearestTab(
    clientX: number,
    _clientY: number
): { tabId: string; side: "left" | "right" } | null {
    let bestTabId: string | null = null;
    let bestDist = Infinity;
    let bestSide: "left" | "right" = "left";

    for (const [tabId, el] of tabWrapperRefs) {
        if (tabId === globalDragTabId) continue;
        const rect = el.getBoundingClientRect();
        const midX = rect.left + rect.width / 2;
        const dist = Math.abs(clientX - midX);
        if (dist < bestDist) {
            bestDist = dist;
            bestTabId = tabId;
            bestSide = clientX < midX ? "left" : "right";
        }
    }
    if (!bestTabId) return null;
    return { tabId: bestTabId, side: bestSide };
}

/**
 * Computes the backend insertion index for ReorderTab (remove-then-insert semantics).
 */
export function computeInsertIndex(
    sourceIndex: number,
    targetIndex: number,
    side: "left" | "right"
): number {
    const rawIndex = side === "left" ? targetIndex : targetIndex + 1;
    return sourceIndex < rawIndex ? rawIndex - 1 : rawIndex;
}
