// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** PeekOverlay's pane-zoom helpers and the panel's width constants. */

import type { JSX } from "solid-js";

/**
 * The narrowest a TALL peek panel ("end" mode) may be, in the panel's own pixels
 * (it scales with pane zoom). A very narrow, very tall panel (a long command made
 * of short lines) looked broken. Placement treats such a panel as at least this
 * wide (so one wider than its row goes left-pinned, over the pane border), and
 * caps it to the room actually available.
 */
export const PEEK_MIN_WIDTH_PX = 600;
/**
 * The minimum width applies only to a panel more than two lines tall, so the
 * small panels (a time/estimate line, a one-line command) stay compact. Measured
 * in panel px at the width the panel would have WITHOUT the minimum. Two lines
 * come to about 45px (a 13px x 1.4 meta line + a 13px x 1.45 body line, the 2px
 * gap, 2+2px padding, 1+1px border); three lines to about 64px. 54 splits them.
 */
export const PEEK_MIN_WIDTH_ABOVE_HEIGHT_PX = 54;

/**
 * This overlay's owning pane's zoom factor, read off the anchor row.
 *
 * `agent-view.tsx` sets BOTH `zoom: <factor>` and `--agent-pane-zoom:
 * <factor>` on the pane root. The row is inside that subtree, so the custom
 * property inherits down to it — reading from the row means this works for
 * whichever pane the hovered entry belongs to (and for the history view,
 * which sets the same pair) without threading a prop through all six
 * call sites.
 *
 * Returns 1 for anything unparseable, absent, or non-positive: a peek
 * rendered outside an agent pane (or in a test harness with no computed
 * custom properties) must keep behaving exactly as it did before.
 */
export function readPaneZoom(row: HTMLElement | undefined): number {
    if (!row) return 1;
    const raw = getComputedStyle(row).getPropertyValue("--agent-pane-zoom").trim();
    const parsed = Number.parseFloat(raw);
    return Number.isFinite(parsed) && parsed > 0 ? parsed : 1;
}

/** Style lengths that are in real viewport px and must be de-scaled. */
const ZOOM_SCALED_LENGTHS = ["left", "top", "width", "max-width", "min-width", "max-height"] as const;

/**
 * Re-express a style computed in REAL viewport pixels so it renders
 * identically on an element that carries `zoom: paneZoom`.
 *
 * This is the whole fix, and the division is the non-obvious half of it.
 * CSS `zoom` on an element multiplies the used value of that element's OWN
 * lengths — including the inset properties of a positioned element. With
 * `zoom: 2`, `left: 100px` paints at 200px from the viewport's left edge.
 * Every input here comes from `getBoundingClientRect()`, which already
 * reports post-zoom rendered pixels, so each length must be pre-divided by
 * the factor to land where it did before.
 *
 * `transform: translateX(-100%)` is deliberately NOT touched: it resolves
 * against the element's own border-box width, which scales with it, so it
 * stays correct at any zoom.
 *
 * A `paneZoom` of exactly 1 returns the style untouched (no `zoom`
 * property emitted at all) — the overwhelmingly common case stays
 * byte-identical to the pre-fix behavior.
 */
export function withPaneZoom(style: JSX.CSSProperties, paneZoom: number): JSX.CSSProperties {
    if (paneZoom === 1) return style;
    const scaled: JSX.CSSProperties = { ...style, zoom: paneZoom };
    for (const key of ZOOM_SCALED_LENGTHS) {
        const value = style[key];
        if (typeof value !== "string") continue;
        const px = Number.parseFloat(value);
        if (!Number.isFinite(px)) continue;
        scaled[key] = `${px / paneZoom}px`;
    }
    return scaled;
}
