// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Mother-window resize on pane tear-off
// (SPEC_PANE_TEAROFF_MOTHER_RESIZE_2026_06_20.md)

/**
 * Shared tear-off helper: try the warm pool first, fall back to the
 * cold-path `openWindowAtPosition` only on rejection. Used by every
 * platform-specific `CrossWindowDragMonitor` variant
 * (win32 / darwin / linux).
 *
 * `tabbar.tsx::performTabTearOff` does NOT use this helper — its
 * tear-off pipeline tracks `coldPathFailed` for F1.B orphan-workspace
 * cleanup safety (codex P1 round-3 #624) and pairs the open with an
 * SC_MOVE handshake. Both flows still try-pool-first, just with
 * different surrounding logic.
 *
 * Pool path is ~0ms first paint; cold path is 150–300ms and goes
 * through `create_isolated_request_context` whose stability issues
 * have been observed to destabilize the source window's renderer
 * post-tearoff. Spec: `docs/specs/SPEC_TEAR_OFF_POOL_PATH_2026_05_06.md`.
 */

import type { getApi } from "@/store/global";
import { Logger } from "@/util/logger";

type Api = ReturnType<typeof getApi>;

// Minimum CSS pixel width the mother window must retain after a pane tear-off
// resize. Below this the remaining layout would be too narrow to be useful.
const MIN_MOTHER_WIDTH = 400;

// Tolerance in CSS px for deciding whether a pane's edge aligns with the
// layout container's edge (sub-pixel rounding, scrollbar gutters, etc.).
const FULL_HEIGHT_EPSILON_PX = 2;

// Default floater size when the source pane element can't be measured
// (e.g. it already unmounted). Used by every platform's pane tear-off.
const DEFAULT_FLOATER_WIDTH = 720;
const DEFAULT_FLOATER_HEIGHT = 480;
// Defensive lower bounds — protect against degenerate (0-width/height)
// rects yielding an unusable floater.
const MIN_FLOATER_WIDTH = 200;
const MIN_FLOATER_HEIGHT = 120;

/**
 * Measure the source pane's rendered size in CSS / DIP pixels.
 *
 * Returned values are LOGICAL (CSS) pixels — do NOT multiply by
 * `window.devicePixelRatio` here. On Windows the floater may spawn on a
 * DIFFERENT monitor (different DPI), so cross-monitor physical scaling
 * MUST happen on the host using `GetDpiForMonitor(MonitorFromPoint(x, y))`
 * against the destination (see `agentmux-cef/src/commands/floating_pane.rs`).
 * On macOS / Linux CEF Views positions in DIP directly, so the host passes
 * these through unscaled.
 *
 * MUST be called BEFORE `TearOffBlock` — that mutation removes the source
 * pane from the layout and unmounts its DOM element. Platform-agnostic;
 * shared by win32 / darwin (/ linux) `CrossWindowDragMonitor` variants.
 */
export function measureSourcePaneSize(blockId: string): { width: number; height: number } {
    const el = document.querySelector(`[data-blockid="${blockId}"]`) as HTMLElement | null;
    if (!el) {
        return { width: DEFAULT_FLOATER_WIDTH, height: DEFAULT_FLOATER_HEIGHT };
    }
    const rect = el.getBoundingClientRect();
    return {
        width: Math.max(MIN_FLOATER_WIDTH, Math.round(rect.width)),
        height: Math.max(MIN_FLOATER_HEIGHT, Math.round(rect.height)),
    };
}

/**
 * Compute the CSS/DIP pixel width the mother window should shrink to after
 * tearing off the pane identified by `blockId`.
 *
 * Returns a width in CSS pixels when ALL of the following hold:
 *   1. The pane element is found in the DOM (must be called BEFORE TearOffBlock).
 *   2. The pane spans the full height of its layout container (top-to-bottom
 *      column — a clean vertical split).
 *   3. The remaining width would be ≥ MIN_MOTHER_WIDTH (400 CSS px).
 *
 * Returns `undefined` when:
 *   - The pane element is missing (unmounted early).
 *   - The pane is in a horizontal split (shares height with siblings) — no resize.
 *   - The remaining width would be too narrow.
 *
 * The value is passed as `mother_resize_to_width` in `open_floating_pane_window`.
 * The host converts it to physical pixels on Windows (using the source window's
 * monitor DPI) and applies it via `SetWindowPos` / CEF `set_bounds`.
 *
 * MUST be called BEFORE `TearOffBlock` — that mutation removes the pane from
 * the layout tree and unmounts its DOM element.
 */
export function measureMotherResize(blockId: string): number | undefined {
    const paneEl = document.querySelector(`[data-blockid="${blockId}"]`) as HTMLElement | null;
    if (!paneEl) return undefined;

    // Resolve the layout display container — the immediate parent grid div.
    // TileLayout renders: <div class="tile-layout"><div class="display-container">…
    // The pane node's DOM element is a descendant of display-container.
    const containerEl = paneEl.closest(".display-container") as HTMLElement | null;
    if (!containerEl) return undefined;

    const paneRect = paneEl.getBoundingClientRect();
    const containerRect = containerEl.getBoundingClientRect();

    // Full-height check: pane must reach from container top to container bottom.
    const spansFullHeight =
        paneRect.top - containerRect.top <= FULL_HEIGHT_EPSILON_PX &&
        containerRect.bottom - paneRect.bottom <= FULL_HEIGHT_EPSILON_PX;

    if (!spansFullHeight) return undefined;

    // Single-pane layout guard: nothing left after resize.
    const newWidth = Math.round(containerRect.width - paneRect.width);
    return newWidth >= MIN_MOTHER_WIDTH ? newWidth : undefined;
}

/**
 * Open a tear-off destination window at `(screenX, screenY)`,
 * preferring the pre-warmed pool. Falls back to cold-path only when
 * `tearOffPoolPromote` rejects (e.g. pool exhausted, host refuses).
 */
export async function openTearOffWindow(
    api: Api,
    newWsId: string,
    screenX: number,
    screenY: number,
    width?: number,
    height?: number,
    tabAnchorX?: number,
    tabAnchorY?: number,
): Promise<void> {
    try {
        await api.tearOffPoolPromote(
            newWsId,
            screenX,
            screenY,
            width,
            height,
            tabAnchorX,
            tabAnchorY,
        );
    } catch (poolErr) {
        Logger.warn("dnd:cross", "pool promote failed, cold-pathing", {
            error: String(poolErr),
        });
        await api.openWindowAtPosition(
            screenX,
            screenY,
            newWsId,
            width,
            height,
            tabAnchorX,
            tabAnchorY,
        );
    }
}
