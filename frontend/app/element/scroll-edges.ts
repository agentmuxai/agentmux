// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * createScrollEdges — whether a horizontally scrolling box has content past
 * its left and right edges, for the edge-fade cue (`scroll-edge-fade` in
 * mixins.scss) on strips with a hidden scrollbar: a pane's tab strip, the
 * window's tab strip.
 *
 * Measured on scroll, on a resize of the box or its first child, and when its
 * children change (tabs added, removed, or resized through their inline
 * style), at most once per frame: reading layout in an observer callback
 * forces it on every frame of a window drag. Call during component setup.
 *
 * With `flushItem` (the selected tab), `flushLeft` says whether that item's left
 * edge sits at the box's left edge while content lies beyond it: the strip drops
 * its left fade there, so the item is not dimmed. `remeasure` asks for a measure
 * when the item changes without a scroll.
 */

import { createSignal, onCleanup, onMount, type Accessor } from "solid-js";

export function createScrollEdges(
    el: () => HTMLElement | undefined,
    flushItem?: () => HTMLElement | undefined
): {
    moreLeft: Accessor<boolean>;
    moreRight: Accessor<boolean>;
    flushLeft: Accessor<boolean>;
    remeasure: () => void;
} {
    const [moreLeft, setMoreLeft] = createSignal(false);
    const [moreRight, setMoreRight] = createSignal(false);
    const [flushLeft, setFlushLeft] = createSignal(false);
    let remeasure = () => {};
    onMount(() => {
        const box = el();
        if (!box) return;
        const measure = () => {
            setMoreLeft(box.scrollLeft > 1);
            setMoreRight(box.scrollLeft + box.clientWidth < box.scrollWidth - 1);
            const item = box.scrollLeft > 1 ? flushItem?.() : undefined;
            // Rects are in viewport pixels, so a zoomed strip tolerates a little more than a pixel.
            setFlushLeft(!!item && Math.abs(item.getBoundingClientRect().left - box.getBoundingClientRect().left) <= 1.5);
        };
        let frame: number | undefined;
        const schedule = () => {
            if (frame == null) frame = requestAnimationFrame(() => { frame = undefined; measure(); });
        };
        remeasure = schedule;
        const ro = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(schedule);
        ro?.observe(box);
        if (box.firstElementChild) ro?.observe(box.firstElementChild);
        const mo = typeof MutationObserver === "undefined" ? undefined : new MutationObserver(schedule);
        mo?.observe(box, { childList: true, subtree: true, attributes: true, attributeFilter: ["style"] });
        box.addEventListener("scroll", schedule, { passive: true });
        measure();
        onCleanup(() => {
            ro?.disconnect();
            mo?.disconnect();
            box.removeEventListener("scroll", schedule);
            if (frame != null) cancelAnimationFrame(frame);
        });
    });
    return { moreLeft, moreRight, flushLeft, remeasure: () => remeasure() };
}
