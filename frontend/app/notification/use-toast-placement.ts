// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Where the toast stack sits: bottom-right, in real pixels, kept off any block a pane or drawer marks
// with `data-toast-avoid` (toast-placement.ts). The stack's gaps are chrome: the status bar they clear
// grows with chrome zoom, so the gaps are the usual ones times `--zoomfactor`.

import { chromeZoomAtom } from "@/app/store/chrome-zoom";
import { createEffect, createSignal, onCleanup, type Accessor } from "solid-js";
import { placeToasts, TOAST_AVOID_ATTR, type Box, type Placement } from "./toast-placement";

// Clear of the status bar (~23px) at the window's bottom edge, plus a gap; at 10px the bottom
// toast sat on top of it.
const USUAL_BOTTOM = 34;
const RIGHT = 58;
const TOP = 40;
const REPLACE_EVERY_MS = 200;

/** The stack's height without the cap placement itself put on it. Measured as it is, a stack shortened to fit above
 *  the status bar would report the shortened height, take the "fits when raised" path on the next measure, lose the
 *  cap and grow back, and the two would alternate. */
export function naturalHeight(stack: HTMLElement): number {
    const body = stack.firstElementChild as HTMLElement | null;
    const cap = body?.style.maxHeight ?? "";
    if (body && cap) body.style.maxHeight = "";
    const height = stack.offsetHeight;
    if (body && cap) body.style.maxHeight = cap;
    return height;
}

/** The stack's distance from the window's right edge, in real pixels. */
export const toastRight = (): number => RIGHT * chromeZoomAtom();

/** Tracks where the stack goes while `active()`. Pass the stack's element to `setStack`, and in `track`
 *  read whatever (the visible toasts) should trigger a fresh measure. */
export function useToastPlacement(active: Accessor<boolean>, track: () => unknown) {
    // The last measured placement and the zoom it was measured at: a placement from another zoom is stale,
    // so until the next measure the stack sits at the usual gap for the current zoom.
    const [measured, setMeasured] = createSignal<Placement & { zoom: number }>({ bottom: USUAL_BOTTOM, zoom: 1 });
    const bottom = () => (measured().zoom === chromeZoomAtom() ? measured().bottom : USUAL_BOTTOM * chromeZoomAtom());
    let stack: HTMLDivElement | undefined;
    // A marked block can change size with no DOM change (the composer's textarea grows as you type), so
    // the marked blocks are watched for resizing too; the set is re-synced at every measure.
    let onMarkedResize = () => {};
    const resizeObserver = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(() => onMarkedResize());
    const watched = new Set<Element>();
    const syncWatched = (marked: Element[]) => {
        for (const el of watched) if (!marked.includes(el)) (resizeObserver?.unobserve(el), watched.delete(el));
        for (const el of marked) if (!watched.has(el)) (resizeObserver?.observe(el), watched.add(el));
    };
    const measure = () => {
        if (!stack) return;
        const marked = [...document.querySelectorAll(`[${TOAST_AVOID_ATTR}]`)];
        syncWatched(marked);
        const avoid: Box[] = marked.map((el) => {
            const r = el.getBoundingClientRect();
            return { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
        });
        const zoom = chromeZoomAtom();
        const next = placeToasts({
            viewportWidth: window.innerWidth,
            viewportHeight: window.innerHeight,
            width: stack.offsetWidth,
            height: naturalHeight(stack),
            right: RIGHT * zoom,
            bottom: USUAL_BOTTOM * zoom,
            top: TOP * zoom,
            avoid,
        });
        const prev = measured();
        if (prev.bottom !== next.bottom || prev.maxHeight !== next.maxHeight || prev.zoom !== zoom) setMeasured({ ...next, zoom });
    };
    // While toasts show: re-place them when the window, a scroll or the page's layout changes, at most
    // every REPLACE_EVERY_MS (streaming text and animations change the DOM constantly, and a persistent
    // toast can stay up for hours), and not for the stack's own style change. A chrome zoom change
    // re-places them too.
    createEffect(() => {
        if (!active()) return;
        track();
        chromeZoomAtom();
        let timer: ReturnType<typeof setTimeout> | undefined;
        const schedule = () => {
            if (timer === undefined) timer = setTimeout(() => ((timer = undefined), measure()), REPLACE_EVERY_MS);
        };
        onMarkedResize = schedule;
        const observer = new MutationObserver((records) => {
            if (records.some((m) => !stack || !stack.contains(m.target))) schedule();
        });
        observer.observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ["class", "style", "hidden", TOAST_AVOID_ATTR] });
        window.addEventListener("resize", schedule);
        window.addEventListener("scroll", schedule, true);
        const first = requestAnimationFrame(measure); // now, so a new toast never shows in the wrong place first
        onCleanup(() => {
            onMarkedResize = () => {};
            resizeObserver?.disconnect();
            watched.clear();
            observer.disconnect();
            window.removeEventListener("resize", schedule);
            window.removeEventListener("scroll", schedule, true);
            clearTimeout(timer);
            cancelAnimationFrame(first);
        });
    });
    /** The stack's body height cap in real pixels, when it has to be shorter to fit above the status bar. */
    const maxHeight = () => (measured().zoom === chromeZoomAtom() ? measured().maxHeight : undefined);
    return { bottom, maxHeight, setStack: (el: HTMLDivElement) => (stack = el) };
}
