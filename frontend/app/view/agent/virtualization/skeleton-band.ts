// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Rows just outside the mounted window, drawn as plain placeholders of the row's own height.
 *
 * The mounted window is a few hundred px either side of the view. When the page is busy, frames come
 * 100 ms or more apart, and a fast scroll takes the view past that edge before a frame has mounted the
 * rows it reached: what the reader sees there is blank. A placeholder is an empty box (no text, no
 * markdown, no measuring), so a band several screens wide costs a few dozen elements, and what is
 * drawn next to a mounted row is a row's shape, not a hole.
 */

import type { LayoutView, RowPosition } from "../../../store/agent-pane-layout/reducer";

/** How far past the mounted window placeholders reach, each side (CSS px). */
export const SKELETON_BAND_PX = 2400;
/** The most placeholders drawn at once, so a document of tiny rows stays cheap. */
export const SKELETON_MAX_ROWS = 80;

/** The rows within `bandPx` beyond the mounted window, nearest the window first on each side. */
export function skeletonRows(view: LayoutView | undefined, bandPx = SKELETON_BAND_PX, max = SKELETON_MAX_ROWS): RowPosition[] {
    if (!view || view.window.endIndex < view.window.startIndex) return [];
    const { rows, window: w } = view;
    const first = rows[w.startIndex];
    const last = rows[w.endIndex];
    if (!first || !last) return [];
    const out: RowPosition[] = [];
    const above = first.start - bandPx;
    const below = last.end + bandPx;
    for (let i = w.startIndex - 1; i >= 0 && rows[i].end > above && out.length < max / 2; i--) out.push(rows[i]);
    for (let i = w.endIndex + 1; i < rows.length && rows[i].start < below && out.length < max; i++) out.push(rows[i]);
    return out;
}
