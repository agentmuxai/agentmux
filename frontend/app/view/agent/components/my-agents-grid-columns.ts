// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * How many columns the My Agents grid has, worked out from its width instead of read back from the browser.
 * Reading `gridTemplateColumns` inside a ResizeObserver forces style and layout on every frame of a window
 * drag; the grid is `repeat(auto-fill, minmax(min(tile, 100%), 1fr))` with a gap (_recent-sessions.scss), so
 * its width, tile minimum and gap give the same count.
 */

/** The tile minimum used when `--agent-session-tile-min` is not set (_recent-sessions.scss). */
const DEFAULT_TILE_MIN_PX = 480;

export interface GridMetrics {
    tile: number;
    gap: number;
}

/** A plain pixel length ("480px", "8.5px"), or null for anything that needs the browser to resolve it. */
function pixels(text: string | undefined): number | null {
    const match = /^\s*(\d+(?:\.\d+)?)px\s*$/.exec(text ?? "");
    return match ? Number(match[1]) : null;
}

/**
 * The grid's tile minimum and gap from its computed style, or null when either is not a plain pixel length
 * (the caller then keeps reading the columns from the browser).
 */
export function gridMetrics(style: Pick<CSSStyleDeclaration, "columnGap" | "getPropertyValue">): GridMetrics | null {
    const declared = style.getPropertyValue?.("--agent-session-tile-min")?.trim();
    const tile = declared ? pixels(declared) : DEFAULT_TILE_MIN_PX;
    const gap = pixels(style.columnGap);
    return tile == null || gap == null ? null : { tile, gap };
}

/** The `auto-fill` track count for a grid `width` px wide: as many minimum-width tracks and gaps as fit, at least one. */
export function columnsForWidth(width: number, { tile, gap }: GridMetrics): number {
    const track = Math.min(tile, width);
    return Math.max(1, Math.floor((width + gap) / (track + gap)));
}
