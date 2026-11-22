// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The element each floating overlay (a tooltip, a popover, a menu) was opened for, so a window capture blacks out an
 * overlay opened from a private pane or panel wherever it is drawn (`capture-mask.ts`). An overlay placed at a point
 * rather than an element is tied to the element under that point. The copy an overlay leaves fading out when it
 * closes keeps its anchor (`EXIT_COPY_ATTR`) until it is removed.
 */

/** Marks the fading copy of an overlay that had an anchor. */
export const EXIT_COPY_ATTR = "data-overlay-exit-copy";

/** The selector of every overlay a window capture's mask looks at. */
export const OVERLAY_SELECTOR = `[data-pane-overlay], [${EXIT_COPY_ATTR}]`;

const anchors = new WeakMap<Element, Element>();

export function setOverlayAnchor(overlay: Element, anchor: Element | DOMRect): void {
    const el = anchor instanceof Element ? anchor : underPoint(overlay, anchor.x + anchor.width / 2, anchor.y + anchor.height / 2);
    if (el) anchors.set(overlay, el);
    else anchors.delete(overlay);
}

/** The topmost element at a point that is not the overlay itself. */
function underPoint(overlay: Element, x: number, y: number): Element | undefined {
    const all = overlay.ownerDocument.elementsFromPoint?.(x, y) ?? [];
    return all.find((e) => !overlay.contains(e));
}

export function overlayAnchor(overlay: Element): Element | undefined {
    return anchors.get(overlay);
}

/** Give `copy`, the fading copy of `overlay`, the overlay's anchor. */
export function keepOverlayAnchor(overlay: Element, copy: Element): void {
    const anchor = anchors.get(overlay);
    if (!anchor) return;
    anchors.set(copy, anchor);
    copy.setAttribute(EXIT_COPY_ATTR, "");
}
