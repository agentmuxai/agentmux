// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Per-window registry of live native browser-pane HWND rects (in CSS
// pixels, matching the overlay-rect coordinate space). Populated by the
// browser view on every create / resize and cleared on close. Read by
// `pane-overlay.ts` to skip the `browser_panes_set_overlay_clip` IPC
// when no overlay actually intersects a pane — most top-bar /
// statusbar / hamburger-menu hovers don't, and the IPC + `SetWindowRgn`
// fan-out on every pane HWND is the dominant cost behind the hover-lag
// felt vs. VS Code (see discussion #1097).
//
// Why CSS pixels and not device pixels: the overlay rects are produced
// from `getBoundingClientRect()` without a dpr multiply, so storing the
// pane rects the same way keeps the intersection check trivial. The
// browser view computes a device-pixel rect for the actual
// `browser_pane_resize` IPC (CEF / SetWindowPos expect physical) and a
// css-pixel rect for this registry — two cheap reads of the same
// `getBoundingClientRect()`.

export interface PaneRect {
    x: number;
    y: number;
    w: number;
    h: number;
}

const paneRects = new Map<string, PaneRect>();

/** Register or update the CSS-pixel rect for a pane HWND. Called by
 *  the browser view after each successful `browser_pane_create` and on
 *  every `syncPosition` tick. Idempotent — overwrites the prior rect. */
export function registerPaneRect(blockId: string, rect: PaneRect): void {
    paneRects.set(blockId, rect);
}

/** Remove a pane from the registry. Called on pane close / dispose so
 *  stale rects don't keep the IPC firing for closed panes. */
export function unregisterPaneRect(blockId: string): void {
    paneRects.delete(blockId);
}

/** Whether any registered pane rect intersects the given overlay rect.
 *  Used by `pane-overlay.ts:sendClip` to bail out when no clip work is
 *  actually needed. AABB-overlap test, no allocation. */
export function anyPaneIntersects(rect: PaneRect): boolean {
    if (rect.w <= 0 || rect.h <= 0) return false;
    const rx2 = rect.x + rect.w;
    const ry2 = rect.y + rect.h;
    for (const p of paneRects.values()) {
        if (p.w <= 0 || p.h <= 0) continue;
        if (rect.x < p.x + p.w && rx2 > p.x && rect.y < p.y + p.h && ry2 > p.y) {
            return true;
        }
    }
    return false;
}

/** Total number of registered panes. The fastest possible bail-out
 *  short-circuit when zero panes are alive (workspaces with no
 *  browser pane — e.g. an editor-only tab). */
export function paneCount(): number {
    return paneRects.size;
}

/** Test-only. Resets the registry between cases. */
export function __resetPaneRectRegistry(): void {
    paneRects.clear();
}
