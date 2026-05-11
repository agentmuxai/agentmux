// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Auto-discovery pane-overlay clipping. See
// docs/specs/SPEC_PANE_OVERLAY_AUTO_CLIP_2026_05_11.md.
//
// Strategy: any DOM element that paints over a CEF browser pane HWND
// tags itself with `data-pane-overlay`. A MutationObserver watches
// the document for tagged elements appearing / disappearing; each
// tagged element gets its own ResizeObserver so its rect re-measures
// when its content changes. Window resize + scroll trigger a sweep.
// All discovered rects flow into `pane-overlay.ts`'s shared map and
// dispatch via the existing `browser_panes_set_overlay_clip` IPC.
//
// Why an attribute, not a hook: discoverable via grep, hard to
// forget, one observer + one map for all overlays. Future overlays
// can't accidentally regress the airspace bug; tag the root div.

import {
    __deleteAutoOverlayRect,
    __rectFromElement,
    __setAutoOverlayRect,
} from "./pane-overlay";

const SELECTOR = "[data-pane-overlay]";

const tracked = new WeakSet<Element>();
const observers = new WeakMap<Element, ResizeObserver>();
const styleObservers = new WeakMap<Element, MutationObserver>();
const trackedList = new Set<Element>();

let started = false;
let sweepScheduled = false;

function scheduleSweep(): void {
    if (sweepScheduled) return;
    sweepScheduled = true;
    requestAnimationFrame(() => {
        sweepScheduled = false;
        for (const el of trackedList) {
            if (!document.body.contains(el)) {
                untrack(el);
                continue;
            }
            updateRect(el);
        }
    });
}

function updateRect(el: Element): void {
    if (!(el instanceof HTMLElement)) return;
    const cs = window.getComputedStyle(el);
    if (
        cs.visibility === "hidden" ||
        cs.display === "none" ||
        parseFloat(cs.opacity) === 0
    ) {
        __deleteAutoOverlayRect(el);
        return;
    }
    __setAutoOverlayRect(el, __rectFromElement(el));
}

function track(el: Element): void {
    if (tracked.has(el)) {
        updateRect(el);
        return;
    }
    tracked.add(el);
    trackedList.add(el);
    const ro = new ResizeObserver(() => updateRect(el));
    ro.observe(el);
    observers.set(el, ro);
    const so = new MutationObserver(() => updateRect(el));
    so.observe(el, { attributes: true, attributeFilter: ["style", "class"] });
    styleObservers.set(el, so);
    updateRect(el);
}

function untrack(el: Element): void {
    if (!tracked.has(el)) return;
    tracked.delete(el);
    trackedList.delete(el);
    observers.get(el)?.disconnect();
    observers.delete(el);
    styleObservers.get(el)?.disconnect();
    styleObservers.delete(el);
    __deleteAutoOverlayRect(el);
}

function trackSubtree(root: Element): void {
    if (root.matches(SELECTOR)) track(root);
    for (const el of root.querySelectorAll(SELECTOR)) track(el);
}

function untrackSubtree(root: Element): void {
    if (tracked.has(root)) untrack(root);
    for (const el of root.querySelectorAll(SELECTOR)) untrack(el);
}

/**
 * Start the auto-clip service. Call once at app startup after the
 * DOM is ready. Idempotent — subsequent calls are no-ops.
 */
export function startPaneOverlayAutoService(): void {
    if (started) return;
    started = true;

    const mo = new MutationObserver((muts) => {
        for (const m of muts) {
            if (m.type === "attributes" && m.target instanceof Element) {
                if (m.target.hasAttribute("data-pane-overlay")) {
                    track(m.target);
                } else {
                    untrack(m.target);
                }
                continue;
            }
            for (const node of m.addedNodes) {
                if (node instanceof Element) trackSubtree(node);
            }
            for (const node of m.removedNodes) {
                if (node instanceof Element) untrackSubtree(node);
            }
        }
    });
    mo.observe(document.body, {
        childList: true,
        subtree: true,
        attributes: true,
        attributeFilter: ["data-pane-overlay"],
    });

    for (const el of document.querySelectorAll(SELECTOR)) track(el);

    window.addEventListener("resize", scheduleSweep, { passive: true });
    window.addEventListener("scroll", scheduleSweep, { passive: true, capture: true });
}
