// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** A rect in device px (the shape `usePaneRectSync` sends the host). */
interface Rect {
    x: number;
    y: number;
    width: number;
    height: number;
}

const sameRect = (a: Rect, b: Rect) => a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;

/** The longest a new page waits for its pane to stop moving. A window that is hidden gets no animation
 *  frames, so this also keeps a page from never being made. About a dozen frames at 60 Hz. */
export const SETTLE_MAX_MS = 200;

/**
 * The pane's rect once it has held still for a frame, for a page that is about to be made.
 *
 * A page is laid out once, at the size it is made with, and again whenever its rect changes. A pane
 * that is mounted while its tile is still taking its place (a split being made, a window opening, a
 * layout rebuilding) measures narrower or shorter than it ends up, and a page made at that size lays
 * out narrow and then jumps wider as the first rect after it lands: a responsive site flips its
 * layout mid-load. So the page waits for two frames that measure the same, non-empty rect, and is
 * made at that. If the rect keeps changing, or no frame comes, it is made at `maxMs` with the latest
 * measurement.
 *
 * `measure` reads the placeholder now. `frame` and `timer` are the page's own `requestAnimationFrame`
 * and `setTimeout`, so tests drive them.
 */
export function settledPaneRect(
    measure: () => Rect,
    options: {
        maxMs?: number;
        frame?: (cb: () => void) => unknown;
        cancelFrame?: (handle: unknown) => void;
        timer?: (cb: () => void, ms: number) => unknown;
        cancelTimer?: (handle: unknown) => void;
    } = {},
): Promise<Rect> {
    const frame = options.frame ?? ((cb) => requestAnimationFrame(cb));
    const cancelFrame = options.cancelFrame ?? ((h) => cancelAnimationFrame(h as number));
    const timer = options.timer ?? ((cb, ms) => setTimeout(cb, ms));
    const cancelTimer = options.cancelTimer ?? ((h) => clearTimeout(h as ReturnType<typeof setTimeout>));
    return new Promise<Rect>((resolve) => {
        let last = measure();
        let pendingFrame: unknown = null;
        let done = false;
        const finish = (rect: Rect) => {
            if (done) return;
            done = true;
            if (pendingFrame != null) cancelFrame(pendingFrame);
            cancelTimer(limit);
            resolve(rect);
        };
        const limit = timer(() => finish(measure()), options.maxMs ?? SETTLE_MAX_MS);
        const step = () => {
            pendingFrame = null;
            if (done) return;
            const now = measure();
            if (now.width > 0 && now.height > 0 && sameRect(now, last)) return finish(now);
            last = now;
            pendingFrame = frame(step);
        };
        pendingFrame = frame(step);
    });
}
