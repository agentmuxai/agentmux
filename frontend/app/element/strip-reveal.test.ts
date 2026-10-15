// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { revealInStrip, revealTargetScrollLeft, STRIP_EDGE_FADE_PX, type StripRevealGeometry } from "./strip-reveal";

// jsdom has no layout, so the strip's and tab's geometry is stubbed: the strip
// shows [0, 300) of 1000px of content; the tab's rect is given relative to
// the strip's left edge at the current scroll offset.
function strip(scrollLeft: number, scrollWidth = 1000, clientWidth = 300, zoom = 1): HTMLElement {
    const el = document.createElement("div");
    Object.defineProperty(el, "scrollWidth", { value: scrollWidth });
    Object.defineProperty(el, "clientWidth", { value: clientWidth });
    Object.defineProperty(el, "offsetWidth", { value: clientWidth });
    el.scrollLeft = scrollLeft;
    let left = scrollLeft;
    Object.defineProperty(el, "scrollLeft", {
        get: () => left,
        set: (v: number) => {
            left = v;
        },
    });
    el.getBoundingClientRect = () => ({ left: 0, right: clientWidth * zoom, width: clientWidth * zoom }) as DOMRect;
    return el;
}

function tab(left: number, right: number): HTMLElement {
    const el = document.createElement("div");
    el.getBoundingClientRect = () => ({ left, right }) as DOMRect;
    return el;
}

describe("revealInStrip", () => {
    it("scrolls to the very end for the last tab, so it shows whole", () => {
        const s = strip(0);
        revealInStrip(s, tab(100, 160), true);
        expect(s.scrollLeft).toBe(700);
    });

    it("scrolls right just enough for a tab past the right edge", () => {
        const s = strip(100);
        revealInStrip(s, tab(280, 340), false);
        expect(s.scrollLeft).toBe(140);
    });

    it("scrolls left just enough for a tab past the left edge", () => {
        const s = strip(400);
        revealInStrip(s, tab(-50, 10), false);
        expect(s.scrollLeft).toBe(350);
    });

    it("converts zoomed rect distances to the strip's own scroll units", () => {
        // At 150% chrome zoom the strip spans 0..450 viewport px; a tab ending
        // 30 viewport px past its edge is 20 of the strip's own px.
        const s = strip(100, 1000, 300, 1.5);
        revealInStrip(s, tab(420, 480), false);
        expect(s.scrollLeft).toBe(120);
    });

    it("leaves a fully visible tab alone", () => {
        const s = strip(200);
        revealInStrip(s, tab(50, 110), false);
        expect(s.scrollLeft).toBe(200);
    });

    it("does nothing when the strip does not scroll", () => {
        const s = strip(0, 300, 300);
        revealInStrip(s, tab(250, 310), true);
        expect(s.scrollLeft).toBe(0);
    });
});

// The pure target: a strip showing [scrollLeft, scrollLeft + 300) of 1000px of
// content, with the item's rect given in viewport px from the strip's left edge.
const FADE = { edgeClearance: STRIP_EDGE_FADE_PX };
function geo(scrollLeft: number, itemLeft: number, itemRight: number, zoom = 1, over: Partial<StripRevealGeometry> = {}): StripRevealGeometry {
    return { scrollLeft, scrollWidth: 1000, clientWidth: 300, itemLeft: itemLeft * zoom, itemRight: itemRight * zoom, zoom, ...over };
}

describe("revealTargetScrollLeft", () => {
    it("leaves an item that is fully inside the fades alone", () => {
        expect(revealTargetScrollLeft(geo(200, 40, 100), false, FADE)).toBeNull();
        expect(revealTargetScrollLeft(geo(200, 28, 272), false, FADE)).toBeNull();
    });

    it("scrolls left to just right of the left fade for an item hidden on the left", () => {
        expect(revealTargetScrollLeft(geo(200, -60, 0), false, FADE)).toBe(200 - 60 - 28);
    });

    it("scrolls right to just left of the right fade for an item hidden on the right", () => {
        expect(revealTargetScrollLeft(geo(200, 320, 380), false, FADE)).toBe(200 + 380 - 300 + 28);
    });

    it("moves an item that sits under the left fade clear of it", () => {
        expect(revealTargetScrollLeft(geo(200, 10, 70), false, FADE)).toBe(200 + 10 - 28);
    });

    it("moves an item that sits under the right fade clear of it", () => {
        expect(revealTargetScrollLeft(geo(200, 230, 290), false, FADE)).toBe(200 + 290 - 300 + 28);
    });

    it("reserves no fade on an edge with nothing beyond it", () => {
        // At the very start there is no left fade; at the end there is no right fade.
        expect(revealTargetScrollLeft(geo(0, 5, 65), false, FADE)).toBeNull();
        expect(revealTargetScrollLeft(geo(700, 240, 298), false, FADE)).toBeNull();
        // Scrolling left lands on 0 rather than a negative offset.
        expect(revealTargetScrollLeft(geo(20, -10, 50), false, FADE)).toBe(0);
        // Scrolling right stops at the end.
        expect(revealTargetScrollLeft(geo(650, 300, 360), false, FADE)).toBe(700);
    });

    it("scrolls to 0 for the first item and to the end for the last", () => {
        expect(revealTargetScrollLeft(geo(300, 0, 0), false, { isFirst: true })).toBe(0);
        expect(revealTargetScrollLeft(geo(0, 0, 0), false, { isFirst: true })).toBeNull();
        expect(revealTargetScrollLeft(geo(300, 0, 0), true, FADE)).toBe(700);
        expect(revealTargetScrollLeft(geo(700, 0, 0), true, FADE)).toBeNull();
    });

    it("does nothing when the strip does not scroll", () => {
        expect(revealTargetScrollLeft(geo(0, 250, 350, 1, { scrollWidth: 300 }), false, FADE)).toBeNull();
        expect(revealTargetScrollLeft(geo(0, 250, 350, 1, { scrollWidth: 300 }), true, FADE)).toBeNull();
    });

    it("shows the start of an item wider than the room", () => {
        expect(revealTargetScrollLeft(geo(200, 100, 400), false, FADE)).toBe(200 + 100 - 28);
    });

    it.each([0.5, 1, 2])("measures in the strip's own pixels at zoom %s", (zoom) => {
        expect(revealTargetScrollLeft(geo(200, 320, 380, zoom), false, FADE)).toBe(200 + 380 - 300 + 28);
        expect(revealTargetScrollLeft(geo(200, -60, 0, zoom), false, FADE)).toBe(200 - 60 - 28);
        expect(revealTargetScrollLeft(geo(200, 40, 100, zoom), false, FADE)).toBeNull();
    });
});

describe("revealInStrip options", () => {
    it("scrolls with `scrollTo` when smooth, and writes nothing else", () => {
        const s = strip(100);
        const scrollTo = vi.fn();
        s.scrollTo = scrollTo as unknown as typeof s.scrollTo;
        revealInStrip(s, tab(280, 340), false, { behavior: "smooth", ...FADE });
        expect(scrollTo).toHaveBeenCalledWith({ left: 100 + 340 - 300 + 28, behavior: "smooth" });
        expect(s.scrollLeft).toBe(100);
    });

    it("writes `scrollLeft` when instant, keeping clear of the fade", () => {
        const s = strip(100);
        revealInStrip(s, tab(280, 340), false, FADE);
        expect(s.scrollLeft).toBe(168);
    });

    it("never calls scrollIntoView", () => {
        const s = strip(100);
        const t = tab(280, 340);
        const into = vi.fn();
        t.scrollIntoView = into;
        revealInStrip(s, t, false, FADE);
        expect(into).not.toHaveBeenCalled();
    });
});
