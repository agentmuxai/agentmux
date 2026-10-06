// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * How a terminal follows a live resize (a window or splitter drag), per
 * docs/specs/SPEC_WINDOW_RESIZE_NO_PAINT_DELAY_2026_09_24.md §4.1.
 *
 * The grid used to refit behind a 50 ms trailing debounce. A ResizeObserver
 * fires every frame during a drag, faster than the debounce, so the grid
 * stayed at its old size for the whole drag and the pane's background showed
 * around it. Now the visual refit follows every frame, the costly column
 * reflow of a long buffer is throttled rather than frozen (VS Code's
 * `TerminalResizeDebouncer`), and only the PTY size message waits for the
 * drag to settle.
 */

export interface GridSize {
    cols: number;
    rows: number;
}

/** A normal buffer longer than this reflows enough on a column change to throttle (VS Code: 200). */
export const LIVE_REFIT_LARGE_BUFFER_LINES = 200;
/** Column refits of a long buffer run at most this often during a drag, leading and trailing. */
export const LARGE_BUFFER_COLS_THROTTLE_MS = 100;
/** The PTY hears the new size this long after the last change (matches usePtyWidth). */
export const PTY_RESIZE_DEBOUNCE_MS = 150;

export type LiveRefit = "none" | "now" | "throttle";

/**
 * What a live resize should do with a proposed grid size.
 * - `none`: the grid already has that size.
 * - `now`: apply it this frame — any row change, or a column change on a short buffer.
 * - `throttle`: a column change on a long buffer; apply through the throttle.
 */
export function liveRefit(proposed: GridSize, current: GridSize, bufferLines: number): LiveRefit {
    if (proposed.cols === current.cols && proposed.rows === current.rows) return "none";
    if (proposed.cols === current.cols) return "now";
    return bufferLines > LIVE_REFIT_LARGE_BUFFER_LINES ? "throttle" : "now";
}
