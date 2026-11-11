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
 * zoom or on how many controls a view adds. An evaluation starts from the full set, so a bar that
 * grows again gets its parts back.
 *
 * Cost. Setting a tier and then measuring makes the browser lay the page out, so the elements of a
 * window are fitted together (`fitAll`): every element's tier is written, then every element is
 * measured, one layout a step however many there are. While a window is resized, an element that
 * is still narrower than its tier below needed is not put back to the full set first: it is only
 * measured, which costs no layout, and is fitted from the full set once the resizing has stopped.
 */

import { createEffect, onCleanup, onMount } from "solid-js";

/** Sub-pixel rounding at a fractional zoom is not an overflow. */
const SLACK_PX = 1;

/** How long after the last size change an element that was only measured is fitted from the full set. */
const SETTLE_MS = 200;

/** One fitted element and what its last fitting found. */
interface Fit {
    el: HTMLElement;
    tiers: readonly string[];
    /** The tier it was left at: 0 is none, n is `tiers[n - 1]`. */
    at: number;
    /** The width its content needed at the tier below `at`, where it did not fit. */
    neededBelow: number | undefined;
}

const fits = new WeakMap<HTMLElement, Fit>();

function fitOf(el: HTMLElement, tiers: readonly string[]): Fit {
    let fit = fits.get(el);
    if (!fit || fit.tiers !== tiers) {
        fit = { el, tiers, at: 0, neededBelow: undefined };
        fits.set(el, fit);
    }
    return fit;
}

const tierName = (fit: Fit, at: number): string | null => (at === 0 ? null : fit.tiers[at - 1]);

/**
 * The width the element's content needs. `scrollWidth` sees only the right side; a row that is
 * pushed to the right edge (`justify-content: flex-end`) overruns on the left, so its children are
 * compared with the element's left edge too. (A `display: contents` child has no box.)
 */
function contentWidth(el: HTMLElement): number {
    if (el.scrollWidth - el.clientWidth > SLACK_PX) return el.scrollWidth;
    const left = el.getBoundingClientRect().left;
    let overrun = 0;
    for (const child of el.children) {
        const box = child.getBoundingClientRect();
        if (box.width > 0) overrun = Math.max(overrun, left - box.left);
    }
    return el.clientWidth + overrun;
}

/** Whether a fitting may start at the element's own tier: the tier below still needs more than it has. */
function stillTooNarrow(fit: Fit): boolean {
    if (fit.at === 0 || fit.neededBelow === undefined) return false;
    if (fit.el.getAttribute("data-fit") !== tierName(fit, fit.at)) return false;
    return fit.neededBelow - fit.el.clientWidth > SLACK_PX;
}

/** Elements measured without being put back to the full set, to be fitted from it once sizes stop changing. */
const unsettled = new Set<Fit>();
let settleTimer: ReturnType<typeof setTimeout> | undefined;

function settleSoon(): void {
    if (settleTimer !== undefined) clearTimeout(settleTimer);
    settleTimer = setTimeout(() => {
        settleTimer = undefined;
        const waiting = [...unsettled].filter((fit) => fit.el.isConnected);
        unsettled.clear();
        fitAll(new Map(waiting.map((fit) => [fit, true])));
    }, SETTLE_MS);
}

/**
 * Fits every element of `work`, a step at a time: each element's tier for the step is written, then
 * each is measured, so a step costs one layout. `true` fits an element from the full set; `false`
 * lets one that is still too narrow for its tier below start at its own tier.
 */
function fitAll(work: Map<Fit, boolean>): void {
    let open: { fit: Fit; at: number }[] = [];
    for (const [fit, fromFullSet] of work) {
        const stay = !fromFullSet && stillTooNarrow(fit);
        if (stay) unsettled.add(fit);
        else unsettled.delete(fit);
        open.push({ fit, at: stay ? fit.at : 0 });
    }
    if (unsettled.size > 0) settleSoon();
    while (open.length > 0) {
        for (const { fit, at } of open) {
            const tier = tierName(fit, at);
            if (fit.el.getAttribute("data-fit") === tier) continue;
            if (tier === null) fit.el.removeAttribute("data-fit");
            else fit.el.setAttribute("data-fit", tier);
        }
        const next: typeof open = [];
        for (const step of open) {
            const { fit, at } = step;
            const needed = contentWidth(fit.el);
            if (needed - fit.el.clientWidth > SLACK_PX && at < fit.tiers.length) {
                fit.neededBelow = needed;
                next.push({ fit, at: at + 1 });
                continue;
            }
            fit.at = at;
            if (at === 0) fit.neededBelow = undefined;
        }
        open = next;
    }
}

/** Sets `el`'s `data-fit` to the first of `tiers` at which it no longer overflows (the last one if none does). */
export function fitToWidth(el: HTMLElement, tiers: readonly string[]): void {
    fitAll(new Map([[fitOf(el, tiers), true]]));
}

/** What the window's observer watches: a fitted element itself, or a part whose size says its content changed. */
const observed = new Map<Element, { fit: Fit; part: boolean }>();
let observer: ResizeObserver | undefined;

/** One callback for every fitted element whose size changed in a frame, so they are fitted together. */
function onSizes(entries: readonly { target: Element }[]): void {
    const work = new Map<Fit, boolean>();
    for (const entry of entries) {
        const watched = observed.get(entry.target);
        if (watched) work.set(watched.fit, (work.get(watched.fit) ?? false) || watched.part);
    }
    fitAll(work);
}

function observe(target: Element, fit: Fit, part: boolean): void {
    observer ??= new ResizeObserver(onSizes);
    observed.set(target, { fit, part });
    observer.observe(target);
}

function unobserve(target: Element): void {
    observed.delete(target);
    observer?.unobserve(target);
    if (observed.size === 0) {
        observer?.disconnect();
        observer = undefined;
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
        const fit = fitOf(el, tiers);
        if (typeof ResizeObserver !== "undefined") {
            const parts = watch(el);
            observe(el, fit, false);
            for (const part of parts) observe(part, fit, true);
            onCleanup(() => {
                unsettled.delete(fit);
                for (const target of [el, ...parts]) unobserve(target);
            });
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
