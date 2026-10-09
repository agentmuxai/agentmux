// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal } from "solid-js";
import { beginDrag, endDrag, isUnderway, markReleased, session, type DragEndReason } from "@/app/drag/drag-session";


/** Half the gap opened on each side of an insertion point (px). Total visual gap = 2 × GAP_PX. */
export const GAP_PX = 12;

// ── Shared drag state ──────────────────────────────────────────────────────

// A window-tab drag lives on the drag session.
// docs/specs/SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md §5.1.

/** `crossWindow` is false for a lone-tab drag, which only the native strip merge may handle. */
export function startWindowTabDrag(tabId: string, wsId: string, crossWindow: boolean): void {
    beginDrag("window-tab", { tabId, wsId }, { crossWindow });
}

/**
 * The source's onDrop. pragmatic fires it for every release, before the tab
 * bar's monitor, so the session is only marked released; the monitor ends it.
 */
export function releaseWindowTabDrag(): void {
    if (session()?.kind === "window-tab") markReleased();
}

export function endWindowTabDrag(reason: DragEndReason): void {
    if (session()?.kind === "window-tab") endDrag(reason);
}

/** The window tab this window is dragging, until its source releases it. Reactive. */
export function draggedWindowTabId(): string | null {
    return isUnderway("window-tab") ? (session()?.source?.tabId ?? null) : null;
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
// spring-loading.
//
// This used to note that it was "deliberately longer than REDOCK_DWELL_MS's
// 180ms" on the grounds that a tab switch is the bigger action. That framing
// was wrong — a redock is the *less* reversible of the two — so do not read
// the current ordering as endorsing it. REDOCK_DWELL_MS has since moved
// independently (180 -> 500 -> 300, the last on how it felt in use); the two
// gate different subsystems and are deliberately separate constants.
// SPEC_FLOATING_PANE_REDOCK_DWELL_2026_09_09.md §3.1.
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
    const dragged = draggedWindowTabId();
    const tabs: { tabId: string; left: number; right: number }[] = [];
    for (const [tabId, el] of tabWrapperRefs) {
        if (tabId === dragged) continue;
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
    const dragged = draggedWindowTabId();

    for (const [tabId, el] of tabWrapperRefs) {
        if (tabId === dragged) continue;
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

// Pixels past the tab strip's bottom edge before a drag becomes a
// tear-off (Chrome uses a similar small threshold). 24 px is enough
// to filter out brief excursions while the user is still hunting for
// the drop position; small enough that the tear feels intentional.
// See docs/specs/SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26 §4.1.
// Pixels past the tab bar's bottom edge before tear-off triggers. Was
// 24px historically, which left a ~24-pixel zone where the user saw
// only the OS drag image with no real window. Lowered to 5 to match
// Chrome's perceived-instant tear-off (just enough to filter trembles).
// Spec: SPEC_TAB_TEAROFF_POSITION_AND_PAINT_2026-05-07.md §4.2.
export const TEAR_PAST_PX = 5;

export type TabRelease = "abort" | "tear-off" | "reorder" | "none";

/**
 * What releasing a window-tab drag does, from the strip's rect and the
 * release point. `ip` is the last computed insertion point: it tracks the
 * cursor's X only, so it can be set even for a release below the strip.
 */
export function decideTabRelease(r: {
    escaped: boolean;
    ip: InsertionPoint | null;
    input: { clientX: number; clientY: number };
    stripRect: { left: number; right: number; top: number; bottom: number } | null;
    tabCount: number;
    draggedTabId: string | null;
    /** The host can open a torn-off tab as its own window (`HostCaps.tearOff`). */
    canTearOff: boolean;
}): TabRelease {
    if (r.escaped) return "abort";
    const { input, stripRect: rect } = r;
    const dropInsideBar =
        rect != null &&
        input.clientY >= rect.top && input.clientY <= rect.bottom &&
        input.clientX >= rect.left && input.clientX <= rect.right;
    // Lone tabs never tear: it would trade one single-tab window for another
    // and strand the source. Their cross-window exit is the host mouse-hook
    // remount.
    const releasedBelowStrip = rect != null && input.clientY > rect.bottom + TEAR_PAST_PX;
    if (r.canTearOff && !dropInsideBar && releasedBelowStrip && r.draggedTabId != null && r.tabCount > 1) return "tear-off";
    if (dropInsideBar && r.ip != null && r.draggedTabId != null) return "reorder";
    return "none";
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
// (CrossWindowDropOverlay's cross-drag-end → MoveTabToWorkspace) can BOTH fire for
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
