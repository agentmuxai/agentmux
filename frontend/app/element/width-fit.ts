// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Narrow chrome that sheds its lowest-priority parts instead of clipping its last one: a pane header
 * keeps Close reachable, the status bar stays one line.
 *
 * The element is stepped through `tiers`, each a value of its `data-fit` attribute, from nothing
 * (everything shown) to the last (the most hidden), and stops at the first whose content fits its
 * width. The stylesheet says what each tier hides. Fitting is judged from the element's own box and
 * its children's, all in the same units at any chrome zoom, so no pixel threshold depends on the
 * zoom or on how many controls a view adds. Every evaluation starts from the full set, so a bar
 * that grows again gets its parts back.
 */

import { createEffect, onCleanup, onMount } from "solid-js";

/** Sub-pixel rounding at a fractional zoom is not an overflow. */
const SLACK_PX = 1;

/**
 * Whether the element's content runs past either side. `scrollWidth` sees only the right side; a row
 * that is pushed to the right edge (`justify-content: flex-end`) overruns on the left, so its
 * children are compared with the element's left edge too. (A `display: contents` child has no box.)
 */
function overflows(el: HTMLElement): boolean {
    if (el.scrollWidth - el.clientWidth > SLACK_PX) return true;
    const left = el.getBoundingClientRect().left - SLACK_PX;
    for (const child of el.children) {
        const box = child.getBoundingClientRect();
        if (box.width > 0 && box.left < left) return true;
    }
    return false;
}

/** Sets `el`'s `data-fit` to the first of `tiers` at which it no longer overflows (the last one if none does). */
export function fitToWidth(el: HTMLElement, tiers: readonly string[]): void {
    for (let i = 0; i <= tiers.length; i++) {
        if (i === 0) el.removeAttribute("data-fit");
        else el.setAttribute("data-fit", tiers[i - 1]);
        if (!overflows(el)) return;
    }
}

/**
 * Fits the element `getEl` returns once mounted, again when it or any of `watch(el)` changes size
 * (the parts whose own width follows what a view puts in them), and after `trigger()` changes (a
 * chrome zoom change resizes nothing the observer sees).
 */
export function useWidthFit(
    getEl: () => HTMLElement | undefined,
    tiers: readonly string[],
    watch: (el: HTMLElement) => Element[] = () => [],
    trigger?: () => unknown,
): void {
    onMount(() => {
        const el = getEl();
        if (!el) return;
        if (typeof ResizeObserver !== "undefined") {
            const observer = new ResizeObserver(() => fitToWidth(el, tiers));
            for (const target of [el, ...watch(el)]) observer.observe(target);
            onCleanup(() => observer.disconnect());
        }
        if (!trigger) return;
        createEffect(() => {
            trigger();
            // The new zoom is written to the page after the signal changes: measure once it is.
            const frame = requestAnimationFrame(() => fitToWidth(el, tiers));
            onCleanup(() => cancelAnimationFrame(frame));
        });
    });
}
