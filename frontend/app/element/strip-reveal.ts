// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Scrolls a horizontally scrolling strip (the window tab bar, a pane's pane tab
// strip, a document tab strip) so one of its items is fully in view
// (docs/specs/SPEC_TAB_BAR_DRAG_GUTTER_2026_10_04.md §3,
// docs/specs/SPEC_PANE_TAB_STRIP_REVEAL_SELECTED_2026_10_15.md §5).
//
// The last item scrolls the strip to its very end, so it shows whole with
// whatever follows it (a separator, the add button). Any other item scrolls
// the least amount that brings it fully into view; a strip that fades its
// edges passes `edgeClearance` so the item lands beside the fade, not under it.
//
// A smooth reveal is a short tween of `scrollLeft` that lasts `durationMs` (the caller passes the
// designed timing through `scaledMs`), not the browser's `behavior: "smooth"`, whose length cannot
// be set or scaled by the Animations setting.
//
// Writes `scrollLeft` or `scrollTo` on the strip only, never `scrollIntoView`:
// that also scrolls every scrollable ancestor, which is how the whole app once
// got shifted sideways (REPORT_LAN_UNDISCOVERABLE_AND_PAGE_SHIFT_2026_10_04.md §3).

/** Width of the `scroll-edge-fade` a pane tab strip draws over each scrollable edge (PaneTabStrip.scss). */
export const STRIP_EDGE_FADE_PX = 28;

export interface StripRevealOptions {
    /** The item is the strip's first: reveal by scrolling back to the start. */
    isFirst?: boolean;
    /** Pixels, in the strip's own units, to keep clear of each edge that has more content beyond it. */
    edgeClearance?: number;
    /** `smooth` animates the scroll over `durationMs`; default `instant`. */
    behavior?: "smooth" | "instant";
    /** How long a smooth scroll takes, in ms (already scaled by the animation setting); 0 or less scrolls at once. */
    durationMs?: number;
    /** A smooth scroll ended, by arriving or by being stopped (`cancelStripScroll`, a newer reveal that has nowhere to scroll).
     *  Not when a newer reveal replaced it: that reveal's own `onSettled` is the one to wait for. */
    onSettled?: () => void;
}

/** What a reveal measures. Lengths are in the strip's own pixels except the two `item*` rects. */
export interface StripRevealGeometry {
    scrollLeft: number;
    scrollWidth: number;
    clientWidth: number;
    /** Viewport-pixel distance from the strip's left edge to the item's left and right edges. */
    itemLeft: number;
    itemRight: number;
    /** Viewport pixels per strip pixel: chrome zoom shows in rects but not in `scrollLeft`. */
    zoom: number;
}

/**
 * The `scrollLeft` that brings an item fully into view, or null when it
 * already is (or the strip does not scroll). Pure.
 */
export function revealTargetScrollLeft(
    g: StripRevealGeometry,
    isLast: boolean,
    { isFirst = false, edgeClearance = 0 }: StripRevealOptions = {}
): number | null {
    const max = g.scrollWidth - g.clientWidth;
    if (max <= 0) return null;
    const pick = (target: number): number | null => {
        const next = Math.min(max, Math.max(0, target));
        return next === g.scrollLeft ? null : next;
    };
    if (isLast) return pick(max);
    if (isFirst) return pick(0);
    const left = g.itemLeft / g.zoom;
    const right = g.itemRight / g.zoom;
    // A fade only covers an edge with more content beyond it.
    const leftFade = g.scrollLeft > 1 ? edgeClearance : 0;
    const rightFade = g.scrollLeft < max - 1 ? edgeClearance : 0;
    if (left < leftFade) return pick(g.scrollLeft + left - edgeClearance);
    if (right > g.clientWidth - rightFade) {
        const toRight = g.scrollLeft + right - g.clientWidth + edgeClearance;
        // An item wider than the room shows its start, not its end.
        return pick(edgeClearance > 0 ? Math.min(toRight, g.scrollLeft + left - edgeClearance) : toRight);
    }
    return null;
}

/** Returns whether it wrote a scroll: false when the item is already in view, or the strip does not scroll. */
export function revealInStrip(strip: HTMLElement, item: HTMLElement, isLast: boolean, options: StripRevealOptions = {}): boolean {
    if (strip.scrollWidth <= strip.clientWidth) return false; // nothing scrolls
    let g: StripRevealGeometry;
    if (isLast || options.isFirst) {
        g = { scrollLeft: strip.scrollLeft, scrollWidth: strip.scrollWidth, clientWidth: strip.clientWidth, itemLeft: 0, itemRight: 0, zoom: 1 };
    } else {
        const stripRect = strip.getBoundingClientRect();
        const itemRect = item.getBoundingClientRect();
        g = {
            scrollLeft: strip.scrollLeft,
            scrollWidth: strip.scrollWidth,
            clientWidth: strip.clientWidth,
            itemLeft: itemRect.left - stripRect.left,
            itemRight: itemRect.right - stripRect.left,
            // The header is drawn with `zoom: var(--zoomfactor)`: rects are in zoomed
            // viewport pixels, `scrollLeft` in the strip's own unzoomed pixels.
            zoom: strip.offsetWidth > 0 ? stripRect.width / strip.offsetWidth : 1,
        };
    }
    const target = revealTargetScrollLeft(g, isLast, options);
    // A newer reveal ends the one in flight where it is, even when it has nowhere to scroll: the tab it asks for may
    // already be in view, and the old target was for another tab.
    if (target === null) {
        cancelStripScroll(strip);
        return false;
    }
    cancelStripScroll(strip, false);
    if (options.behavior === "smooth" && (options.durationMs ?? 0) > 0) tweenScrollLeft(strip, target, options.durationMs!, options.onSettled);
    else strip.scrollLeft = target;
    return true;
}

const tweens = new WeakMap<HTMLElement, { frame: number; onSettled?: () => void }>();

/** Stops a smooth reveal of `strip` where it is, and says so to its `onSettled` unless `settle` is false (a replacement is taking over). Reports whether one was running. */
export function cancelStripScroll(strip: HTMLElement, settle = true): boolean {
    const running = tweens.get(strip);
    if (!running) return false;
    cancelAnimationFrame(running.frame);
    tweens.delete(strip);
    if (settle) running.onSettled?.();
    return true;
}

/** Decelerating: most of the distance in the first half, a soft landing. */
const easeOut = (t: number): number => 1 - (1 - t) ** 3;

function tweenScrollLeft(strip: HTMLElement, target: number, durationMs: number, onSettled?: () => void): void {
    const from = strip.scrollLeft;
    let start: number | undefined;
    const step = (now: number) => {
        start ??= now;
        const t = Math.min(1, (now - start) / durationMs);
        strip.scrollLeft = t >= 1 ? target : from + (target - from) * easeOut(t);
        if (t < 1) {
            tweens.set(strip, { frame: requestAnimationFrame(step), onSettled });
            return;
        }
        tweens.delete(strip);
        onSettled?.();
    };
    tweens.set(strip, { frame: requestAnimationFrame(step), onSettled });
}
