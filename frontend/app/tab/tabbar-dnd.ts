// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal } from "solid-js";
import { atoms } from "@/app/store/global";
// Direct submodule imports (NOT the @/layout/index barrel — that barrel
// re-exports TileLayout, and TileLayout.win32.tsx imports cleanupTileDragState
// below, which would create an import cycle).
import { getLayoutModelForTabById } from "@/layout/lib/layoutModelHooks";
import { pruneDanglingLeaves } from "@/layout/lib/layoutPersistence";
import { setTileDragInFlight } from "@/layout/lib/dragInFlight";
import { clearCrossTabDrop } from "@/layout/lib/crossTabDrag";

export const tabItemType = "TAB_ITEM";

/** Half the gap opened on each side of an insertion point (px). Total visual gap = 2 × GAP_PX. */
export const GAP_PX = 12;

// Pixels past the tab bar's bottom edge before tear-off triggers. Chrome-
// style "perceived-instant" tear-off — just enough to filter trembles.
// Shared between tab-reorder.ts (non-Windows pragmatic-dnd path) and
// droppable-tab.tsx (Windows native-pointer-drag path, SPEC_NATIVE_POINTER_
// DRAG_TEAROFF_2026_07_28.md) so both platforms tear off at the same threshold.
// See SPEC_TAB_TEAROFF_POSITION_AND_PAINT_2026-05-07.md §4.2.
export const TEAR_PAST_PX = 5;

// ── Shared drag state ──────────────────────────────────────────────────────

export let globalDragTabId: string | null = null;
export function setGlobalDragTabId(id: string | null): void {
    globalDragTabId = id;
}

// Set true if Escape is pressed at any point during the current tab drag,
// reset false at drag start. Checked by tab-reorder.ts's onDrop BEFORE the
// tear-off/reorder decision — pragmatic-drag-and-drop's underlying HTML5
// drag session does not itself cancel on Escape (that's a native-OLE-drag
// behavior some platforms have, not a web DnD standard guarantee), so
// without this flag Escape has no effect on the commit-on-release outcome:
// the drag simply continues, and releasing below the strip still tears the
// tab off into a new window regardless of the Escape keypress in between.
// Keyboard events, unlike mouse events, ARE reliably delivered to the page
// during an active HTML5 drag, so a plain `keydown` listener is sufficient
// here — no native hook needed (this fix is intentionally cross-platform,
// not scoped to macOS's CGEventTap hook, which only affects cross-window
// merge-candidate detection and has no bearing on this decision either).
export let dragEscaped = false;
export function setDragEscaped(v: boolean): void {
    dragEscaped = v;
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

    // Threshold on each remaining tab's CENTER, not the inter-tab gap.
    //
    // The dragged tab stays in the strip at opacity 0.35 (it is not
    // collapsed — see tabbar.scss `.tab-dragging`) but is excluded from
    // `tabs` above. A gap-midpoint approach therefore measured the cursor
    // against the empty *space the dragged tab still occupies*, which made
    // the "cross to move" threshold land ~1.5–2 tab-widths away and depend
    // on where the tab was grabbed — the reported "drag across 1 tab
    // doesn't move, across 2 moves 1" / "too tight" symptom.
    //
    // Center-thresholding is the standard tab-reorder rule: the insertion
    // lands before the first remaining tab whose center is to the RIGHT of
    // the cursor; if the cursor is past every center, it appends after the
    // last tab. Crossing a neighbour's center (~1 tab of travel from an
    // adjacent grab) commits the move, which matches the natural mental
    // model.
    for (let i = 0; i < tabs.length; i++) {
        const center = (tabs[i].left + tabs[i].right) / 2;
        if (clientX < center) {
            return {
                beforeTabId: i === 0 ? null : tabs[i - 1].tabId,
                afterTabId: tabs[i].tabId,
            };
        }
    }
    // Past every center → after the last tab.
    return { beforeTabId: tabs[tabs.length - 1].tabId, afterTabId: null };
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

/**
 * Convert an insertion point to a numeric index into `tabs` for a tab
 * arriving from ANOTHER workspace (cross-window merge / remount). The
 * incoming tab isn't in `tabs`, so no removal-shift adjustment is needed
 * (unlike computeInsertIndex above, which handles in-strip reorders).
 * A null insertion point (or unknown afterTabId) appends at the end.
 */
export function insertionPointToIndex(ip: InsertionPoint | null, tabs: string[]): number {
    if (!ip) return tabs.length;
    if (ip.beforeTabId === null) return 0;
    if (ip.afterTabId === null) return tabs.length;
    const idx = tabs.indexOf(ip.afterTabId);
    return idx < 0 ? tabs.length : idx;
}

// ── Cross-window merge dedup ────────────────────────────────────────────────
// A direct cross-window tab remount (tabdrag:merge-direct, emitted by the
// host's mouse hook) and the legacy HTML5 cross-drag pipeline
// (DragOverlay's cross-drag-end → MoveTabToWorkspace) can BOTH fire for
// one gesture — the hook resolves on WM_LBUTTONUP while the source
// window's dragend independently drives the cross-drag pipeline. Both
// handlers run in the TARGET window, so a same-context recency mark is
// enough to make the second one a no-op. The backend would reject the
// duplicate anyway (the tab has already left its claimed source
// workspace), but deduping here avoids a guaranteed error-log per merge.

const MERGE_DEDUP_WINDOW_MS = 5000;
const recentTabMerges = new Map<string, number>();

export function markTabMerged(tabId: string, now: number = Date.now()): void {
    recentTabMerges.set(tabId, now);
}

export function wasTabRecentlyMerged(tabId: string, now: number = Date.now()): boolean {
    const at = recentTabMerges.get(tabId);
    if (at === undefined) return false;
    if (now - at > MERGE_DEDUP_WINDOW_MS) {
        recentTabMerges.delete(tabId);
        return false;
    }
    return true;
}

// ── Pane (tile) drag end-of-gesture cleanup ─────────────────────────────────
// End-of-drag cleanup for a pane (tile) drag, wherever the release happens:
// stop any pending spring switch, kill the hover flash, drop any
// un-consumed cross-tab record, and deactivate the overlay of every tab the
// drag spring-switched through (their activeDrag was force-set by the
// spring-switch hit-test; TileLayout's own cleanup only covers the SOURCE
// tab's model). Shared by tab-reorder.ts's pragmatic-dnd tile monitor
// (macOS/Linux, and any in-window Windows drop that still round-trips
// through it) and TileLayout.win32.tsx's native pointer-drag tracker
// (Windows pane drag source — SPEC_NATIVE_POINTER_DRAG_TEAROFF_2026_07_28
// §3.5), which has no pragmatic-dnd monitor to dispatch this anymore.
//
// A stuck activeDrag is a DEAD TAB — the overlay-container sits over the
// entire tile area with pointer-events:auto and eats every click.
export function cleanupTileDragState(): void {
    setHoveredDropTabId(null);
    clearCrossTabDrop();
    setTileDragInFlight(false);
    const ws = atoms.workspace();
    const allTabIds = [...(ws?.pinnedtabids ?? []), ...(ws?.tabids ?? [])];
    for (const tabId of allTabIds) {
        getLayoutModelForTabById(tabId)?.activeDrag._set(false);
    }
    dragActivatedTabIds.clear();
    // Deferred dangling-leaf prune: mid-drag pruning is gated off (see
    // pruneDanglingLeaves), so the source tab's disowned leaf is removed
    // HERE, after the gesture and the move RPC's Tab updates have settled.
    // 250ms comfortably covers the observed 20-40ms RPC round-trip.
    setTimeout(() => {
        for (const tabId of allTabIds) {
            const model = getLayoutModelForTabById(tabId);
            if (model) pruneDanglingLeaves(model);
        }
    }, 250);
}
