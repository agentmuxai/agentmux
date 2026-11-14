// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The pane header's one horizontal scroll range (docs/specs/SPEC_PANE_HEADER_SCROLL_RANGE_2026_11_10.md).

import { onCleanup, onMount, type Accessor } from "solid-js";
import { createScrollEdges } from "@/app/element/scroll-edges";
import { registerStripScroller } from "@/app/element/strip-scroller";
import { stripWheelDelta } from "@/app/element/strip-wheel";

/** Width of the fade the range draws over a scrollable edge (`scroll-edge-fade(20px)` in block.scss). */
const HEADER_RANGE_FADE_PX = 20;

/** The header's one horizontal scroll range (#29325): its element, its edge fades, and the wheel
 *  that scrolls it sideways over the whole header row. A horizontal gesture is the range's own
 *  (its overflow is native); a vertical one over the row must not scroll the page under it.
 *
 *  The tab strip inside does not scroll itself: its reveal scrolls the range (strip-scroller.ts), so
 *  the active tab comes into view, clear of the fade, when it is selected, when the pane narrows, and
 *  when the person focuses the pane or works in it; their own scroll of the range is told to it here.
 *  Called once, in the header's own component. */
export function useHeaderScrollRange(header: () => HTMLElement | undefined): {
    setRange: (el: HTMLDivElement) => void;
    moreLeft: Accessor<boolean>;
    moreRight: Accessor<boolean>;
} {
    let range: HTMLDivElement | undefined;
    const edges = createScrollEdges(() => range);
    const userScrollListeners = new Set<() => void>();
    onMount(() => {
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
                })
            );
        },
        moreLeft: edges.moreLeft,
        moreRight: edges.moreRight,
    };
}
