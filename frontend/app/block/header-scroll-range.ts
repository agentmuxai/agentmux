// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The pane header's one horizontal scroll range (docs/specs/SPEC_PANE_HEADER_SCROLL_RANGE_2026_11_10.md).

import { onCleanup, onMount, type Accessor } from "solid-js";
import { PANE_ROOT_SELECTOR } from "@/app/element/pane-strip-registry";
import { createScrollEdges } from "@/app/element/scroll-edges";
import { registerStripScroller } from "@/app/element/strip-scroller";
import { stripWheelDelta } from "@/app/element/strip-wheel";

/** Width of the fade the range draws over a scrollable edge (`scroll-edge-fade(20px)` in block.scss). */
const HEADER_RANGE_FADE_PX = 20;

/** Where each pane's header range was last scrolled to, by the pane's root element. */
const lastScrollLeft = new WeakMap<Element, number>();

/**
 * A pane draws a new header for each tab it shows, so a selection gives it a new range, at its start. This
 * puts the new range where the pane's last one was, before it is first painted, and says so (`onCarried`):
 * the tab the person picked then travels by the least it takes from what they were looking at, as in a
 * strip that stays. A header's first range in a pane has nothing to take over.
 */
function carryScrollPosition(range: HTMLElement, onCarried: () => void): void {
    const root = range.closest(PANE_ROOT_SELECTOR);
    if (!root) return;
    const carried = lastScrollLeft.get(root) ?? 0;
    if (carried > 0) {
        range.scrollLeft = carried;
        onCarried();
    }
    const remember = () => lastScrollLeft.set(root, range.scrollLeft);
    range.addEventListener("scroll", remember, { passive: true });
    onCleanup(() => {
        range.removeEventListener("scroll", remember);
        // A scroll in the header's last frame has sent no event yet. A range already out of the page reads 0.
        if (range.isConnected) remember();
    });
}

/** The header's one horizontal scroll range (#29325): its element, its edge fades, and the wheel
 *  that scrolls it sideways over the whole header row. A horizontal gesture is the range's own
 *  (its overflow is native); a vertical one over the row must not scroll the page under it.
 *
 *  The tab strip inside does not scroll itself: its reveal scrolls the range (strip-scroller.ts), so
 *  the active tab comes into view, clear of the fade, when it is selected, when the pane narrows, and
 *  when the person focuses the pane or works in it; their own scroll of the range is told to it here.
 *  The range starts where the pane's last header was scrolled to (`carryScrollPosition`).
 *  Called once, in the header's own component. */
export function useHeaderScrollRange(header: () => HTMLElement | undefined): {
    setRange: (el: HTMLDivElement) => void;
    moreLeft: Accessor<boolean>;
    moreRight: Accessor<boolean>;
} {
    let range: HTMLDivElement | undefined;
    const edges = createScrollEdges(() => range);
    const userScrollListeners = new Set<() => void>();
    const carriedListeners = new Set<() => void>();
    let carried = false;
    onMount(() => {
        if (range) {
            carryScrollPosition(range, () => {
                carried = true;
                for (const fn of carriedListeners) fn();
            });
        }
        const el = header();
        if (!el) return;
        // The person's own scroll, when there is something to scroll: it stands until the next reveal.
        const userScrolls = (): boolean => {
            if (!range || range.scrollWidth <= range.clientWidth) return false;
            for (const fn of userScrollListeners) fn();
            return true;
        };
        // A real listener, not Solid's onWheel: the delegated one is passive, so preventDefault() would be a no-op.
        const onWheel = (e: WheelEvent) => {
            if (!range || !userScrolls()) return;
            if (Math.abs(e.deltaX) >= Math.abs(e.deltaY)) return;
            e.preventDefault();
            range.scrollLeft += stripWheelDelta(e.deltaY, e.deltaMode);
        };
        el.addEventListener("wheel", onWheel, { passive: false });
        el.addEventListener("touchmove", userScrolls, { passive: true });
        onCleanup(() => {
            el.removeEventListener("wheel", onWheel);
            el.removeEventListener("touchmove", userScrolls);
        });
    });
    return {
        setRange: (el) => {
            range = el;
            onCleanup(
                registerStripScroller(el, {
                    overflows: () => edges.moreLeft() || edges.moreRight(),
                    edgeFadePx: HEADER_RANGE_FADE_PX,
                    onUserScroll: (fn) => {
                        userScrollListeners.add(fn);
                        return () => userScrollListeners.delete(fn);
                    },
                    onCarried: (fn) => {
                        // Whichever of the strip and the header mounts first.
                        if (carried) fn();
                        carriedListeners.add(fn);
                        return () => carriedListeners.delete(fn);
                    },
                })
            );
        },
        moreLeft: edges.moreLeft,
        moreRight: edges.moreRight,
    };
}
