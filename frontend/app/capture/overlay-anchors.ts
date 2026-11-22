// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The element each floating overlay (a tooltip, a popover, a menu) was opened for, so a window capture blacks out an
 * overlay opened from a private pane or panel wherever it is drawn (`capture-mask.ts`). An overlay placed at a point
 * rather than an element is tied to the element under that point.
 */

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
