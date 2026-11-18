// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Measuring the panel must not move what the person has scrolled in it. The measure lifts the panel's height cap, and
 * a browser clamps the panel's scroll offset to 0 the moment the panel is as tall as its content (the box has nothing
 * to scroll): a re-placement, which runs on every scroll of the transcript, sent a panel the person was reading back
 * to its top. jsdom has no layout, so the browser's clamp is played by the test.
 */

import { describe, expect, it } from "vitest";
import { createPeekMeasure } from "./peek-overlay-measure";
import type { PeekState } from "./peek-overlay-state";

/** A panel whose scroll offset is cleared by layout whenever its max-height is lifted, as a browser does. */
function scrolledPanel(scrollTop: number) {
    const el = document.createElement("div");
    let top = scrollTop;
    Object.defineProperty(el, "scrollTop", { get: () => top, set: (v: number) => (top = v), configurable: true });
    el.getBoundingClientRect = () => {
        if (el.style.maxHeight === "none") top = 0; // laid out at its full height: nothing to scroll
        return { width: 300, height: 900, top: 0, left: 0, right: 300, bottom: 900, x: 0, y: 0, toJSON: () => ({}) } as DOMRect;
    };
    el.style.maxHeight = "500px";
    return el;
}

const measureOf = (el: HTMLElement) => createPeekMeasure({ floatingEl: el } as PeekState);

describe("createPeekMeasure keeps the panel's own scroll position", () => {
    it("after measuring its height", () => {
        const el = scrolledPanel(120);
        expect(measureOf(el).measureHeightAt(700, 0, 1)).toBe(900);
        expect(el.scrollTop).toBe(120);
        expect(el.style.maxHeight).toBe("500px");
    });

    it("after measuring its width", () => {
        const el = scrolledPanel(75);
        expect(measureOf(el).measureWidth()).toBe(300);
        expect(el.scrollTop).toBe(75);
    });

    it("leaves a panel at its top alone", () => {
        const el = scrolledPanel(0);
        measureOf(el).measureHeightAt(700, 0, 1);
        expect(el.scrollTop).toBe(0);
    });
});
