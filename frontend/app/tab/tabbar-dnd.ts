// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal } from "solid-js";

export const tabItemType = "TAB_ITEM";

// ── Shared drag state ──────────────────────────────────────────────────────
// Module-level singletons shared between DroppableTab instances and TabBar's
// monitorForElements. Safe for a single-webview app; encapsulate into a class
// when multi-window support lands.

export let globalDragTabId: string | null = null;

export function setGlobalDragTabId(id: string | null): void {
    globalDragTabId = id;
}

// Nearest-tab hint: when the cursor is outside all drop targets the monitor
// sets this so the closest DroppableTab shows an insertion indicator.
export const [nearestHint, setNearestHint] = createSignal<{
    tabId: string;
    side: "left" | "right";
} | null>(null);

// Registry of tab wrapper elements, keyed by tabId, for nearest-tab computation.
export const tabWrapperRefs = new Map<string, HTMLDivElement>();

// ── Utilities ──────────────────────────────────────────────────────────────

/**
 * Returns the tab closest to the cursor (by horizontal midpoint distance),
 * excluding the tab currently being dragged.
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
 * Computes the insertion index to pass to ReorderTab, accounting for the
 * backend's remove-then-insert behaviour on section-specific arrays.
 *
 * When the source tab sits before the target in the array, the array shrinks
 * by 1 after removal, so the raw insertion index must be decremented by 1 to
 * land in the correct slot.
 *
 * Both `sourceIndex` and `targetIndex` must be section-relative (i.e. indices
 * within `tabids[]` or `pinnedtabids[]`, NOT the combined allTabIds array).
 */
export function computeInsertIndex(
    sourceIndex: number,
    targetIndex: number,
    side: "left" | "right"
): number {
    const rawIndex = side === "left" ? targetIndex : targetIndex + 1;
    return sourceIndex < rawIndex ? rawIndex - 1 : rawIndex;
}
