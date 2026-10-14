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
 */

import { createSignal, onCleanup, onMount, type Accessor } from "solid-js";

export function createScrollEdges(el: () => HTMLElement | undefined): {
    moreLeft: Accessor<boolean>;
    moreRight: Accessor<boolean>;
} {
    const [moreLeft, setMoreLeft] = createSignal(false);
    const [moreRight, setMoreRight] = createSignal(false);
    onMount(() => {
        const box = el();
        if (!box) return;
        const measure = () => {
            setMoreLeft(box.scrollLeft > 1);
            setMoreRight(box.scrollLeft + box.clientWidth < box.scrollWidth - 1);
        };
        let frame: number | undefined;
        const schedule = () => {
            if (frame == null) frame = requestAnimationFrame(() => { frame = undefined; measure(); });
        };
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
    return { moreLeft, moreRight };
}
