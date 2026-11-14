// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A box that scrolls a tab strip from outside it: the pane header's one scroll range
// (docs/specs/SPEC_PANE_HEADER_SCROLL_RANGE_2026_11_10.md), which holds the strip, its "+" and the
// header's controls. A strip inside such a box does not scroll itself, so its reveal
// (strip-reveal-controller.ts) scrolls the box. Keyed by DOM, as pane-strip-registry.ts is: the
// strip is built before the header that holds it.

import type { Accessor } from "solid-js";

export interface StripScroller {
    /** Whether the box has content past either edge. */
    overflows: Accessor<boolean>;
    /** Width of the fade the box draws over an edge with more content beyond it. */
    edgeFadePx: number;
    /** Calls `fn` when the person scrolls the box themselves (a wheel, a touch). Returns the removal. */
    onUserScroll(fn: () => void): () => void;
    /** Calls `fn` when the box starts scrolled to where the box before it was (a pane draws a new header for
     *  each tab it shows): the reveal that follows is a selection's. Returns the removal. */
    onCarried(fn: () => void): () => void;
}

const SCROLLER_ATTR = "data-strip-scroller";

const scrollers = new WeakMap<Element, StripScroller>();

/** Makes `el` the box that scrolls the strips inside it. Returns the removal. */
export function registerStripScroller(el: HTMLElement, scroller: StripScroller): () => void {
    scrollers.set(el, scroller);
    el.setAttribute(SCROLLER_ATTR, "");
    return () => {
        if (scrollers.get(el) !== scroller) return;
        scrollers.delete(el);
        el.removeAttribute(SCROLLER_ATTR);
    };
}

/** The box that scrolls `strip` from outside it, when it sits in one. Reads no layout. */
export function outerStripScroller(strip: Element | undefined): { el: HTMLElement; scroller: StripScroller } | undefined {
    const el = strip?.closest<HTMLElement>(`[${SCROLLER_ATTR}]`);
    const scroller = el ? scrollers.get(el) : undefined;
    return el && scroller ? { el, scroller } : undefined;
}
