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

/**
 * At most this many terminals refit in one frame of a live resize. A refit
 * reallocates the terminal's WebGL canvas, which waits on the GPU process, and
 * with every terminal refitting in the same frame a window drag over a tab of
 * four of them missed every other frame
 * (ANALYSIS_WINDOW_RESIZE_REPAINT_LAG_2026_10_06.md §12).
 */
export const LIVE_REFITS_PER_FRAME = 2;

export interface RefitScheduler {
    /** Run `job` this frame if the frame has room, otherwise on a later frame. One job per key. */
    schedule(key: object, job: () => void): void;
    /** Drop a key's waiting job, e.g. when its terminal is disposed. */
    cancel(key: object): void;
}

/**
 * Spreads terminal refits over frames: up to `perFrame` run in the frame
 * they're asked for, the rest wait for the next frames in arrival order. A
 * waiting job is replaced by a newer one for the same key, keeping its place,
 * and runs then, so it measures the size of that frame.
 */
export function createRefitScheduler(
    perFrame: number,
    requestFrame: (cb: () => void) => void = (cb) => requestAnimationFrame(cb)
): RefitScheduler {
    const waiting = new Map<object, () => void>();
    let ranThisFrame = 0;
    let frameRequested = false;

    const onFrame = () => {
        frameRequested = false;
        ranThisFrame = 0;
        drain();
    };
    // Counting is per frame, so a new frame resets it; also drains what waits.
    const requestNextFrame = () => {
        if (frameRequested) return;
        frameRequested = true;
        requestFrame(onFrame);
    };
    // A job may schedule again (the long-buffer column throttle does); the
    // running loop picks that up, since a Map's iteration visits new entries.
    let draining = false;
    const drain = () => {
        draining = true;
        try {
            for (const [key, job] of waiting) {
                if (ranThisFrame >= perFrame) break;
                waiting.delete(key);
                ranThisFrame++;
                job();
            }
        } finally {
            draining = false;
        }
        if (waiting.size > 0 || ranThisFrame > 0) requestNextFrame();
    };

    return {
        schedule(key, job) {
            waiting.set(key, job);
            if (!draining) drain();
        },
        cancel(key) {
            waiting.delete(key);
        },
    };
}

/** The one scheduler every terminal's live refit goes through. */
export const liveRefits = createRefitScheduler(LIVE_REFITS_PER_FRAME);
