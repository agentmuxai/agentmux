// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { revealTabInStrip } from "./tab-strip-scroll";

// jsdom has no layout, so the strip's and tab's geometry is stubbed: the strip
// shows [0, 300) of 1000px of content; the tab's rect is given relative to
// the strip's left edge at the current scroll offset.
function strip(scrollLeft: number, scrollWidth = 1000, clientWidth = 300): HTMLElement {
    const el = document.createElement("div");
    Object.defineProperty(el, "scrollWidth", { value: scrollWidth });
    Object.defineProperty(el, "clientWidth", { value: clientWidth });
    el.scrollLeft = scrollLeft;
    let left = scrollLeft;
    Object.defineProperty(el, "scrollLeft", {
        get: () => left,
        set: (v: number) => {
            left = v;
        },
    });
    el.getBoundingClientRect = () => ({ left: 0, right: clientWidth }) as DOMRect;
    return el;
}

function tab(left: number, right: number): HTMLElement {
    const el = document.createElement("div");
    el.getBoundingClientRect = () => ({ left, right }) as DOMRect;
    return el;
}

describe("revealTabInStrip", () => {
    it("scrolls to the very end for the last tab, so the drag square shows", () => {
        const s = strip(0);
        revealTabInStrip(s, tab(100, 160), true);
        expect(s.scrollLeft).toBe(700);
    });

    it("scrolls right just enough for a tab past the right edge", () => {
        const s = strip(100);
        revealTabInStrip(s, tab(280, 340), false);
        expect(s.scrollLeft).toBe(140);
    });

    it("scrolls left just enough for a tab past the left edge", () => {
        const s = strip(400);
        revealTabInStrip(s, tab(-50, 10), false);
        expect(s.scrollLeft).toBe(350);
    });

    it("leaves a fully visible tab alone", () => {
        const s = strip(200);
        revealTabInStrip(s, tab(50, 110), false);
        expect(s.scrollLeft).toBe(200);
    });

    it("does nothing when the strip does not scroll", () => {
        const s = strip(0, 300, 300);
        revealTabInStrip(s, tab(250, 310), true);
        expect(s.scrollLeft).toBe(0);
    });
});
