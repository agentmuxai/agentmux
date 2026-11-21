// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The windowed card list: a short one is drawn whole, a long one draws the cards near the view between two
// spacers that keep its length, and a scroll moves the window.

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { WINDOW_MIN, WindowedList, windowRange } from "./kanban-window";

describe("windowRange", () => {
    // Four cards of 10, 20, 10 and 10 pixels.
    const starts = [0, 10, 30, 40, 50];

    it("keeps the cards that meet the band", () => {
        expect(windowRange(starts, 12, 35)).toEqual({ first: 1, last: 3 });
    });

    it("takes the whole list when the band covers it", () => {
        expect(windowRange(starts, -100, 1000)).toEqual({ first: 0, last: 4 });
    });

    it("keeps one card when the band is past the end or before the start", () => {
        expect(windowRange(starts, 500, 600)).toEqual({ first: 3, last: 4 });
        expect(windowRange(starts, -50, -10)).toEqual({ first: 0, last: 1 });
    });

    it("keeps nothing for an empty list", () => {
        expect(windowRange([0], 0, 100)).toEqual({ first: 0, last: 0 });
    });
});

describe("WindowedList", () => {
    let scrollTop = 0;
    beforeEach(() => {
        scrollTop = 0;
        // The frame callback runs at once, so a scroll is one step.
        vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
            cb(0);
            return 0;
        });
        // jsdom lays nothing out: the scroller is 500 high at the top of the screen and the list moves up as it scrolls.
        vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
            const top = this.classList.contains("scroller") ? 0 : this.classList.contains("kanban-window") ? -scrollTop : 0;
            return { top, bottom: top + (this.classList.contains("scroller") ? 500 : 0), left: 0, right: 0, width: 0, height: 0 } as DOMRect;
        });
    });
    afterEach(() => {
        cleanup();
        vi.restoreAllMocks();
        vi.unstubAllGlobals();
    });

    const items = (n: number) => Array.from({ length: n }, (_, i) => ({ id: `t${i}` }));
    function list(n: number) {
        return render(() => (
            <div class="scroller" style={{ "overflow-y": "auto" }}>
                <WindowedList items={items(n)} keyOf={(t) => t.id} each={(t) => <span class="row">{t.id}</span>} />
            </div>
        ));
    }
    const rows = (c: HTMLElement) => [...c.querySelectorAll(".row")].map((r) => r.textContent);
    const spacers = (c: HTMLElement) => [...c.querySelectorAll<HTMLElement>(".kanban-window__spacer")].map((s) => s.style.height);

    it("draws a short list whole, with no window around it", () => {
        const { container } = list(WINDOW_MIN);
        expect(rows(container)).toHaveLength(WINDOW_MIN);
        expect(container.querySelector(".kanban-window")).toBeNull();
    });

    it("draws only the cards near the view of a long list, and a spacer for the rest", () => {
        const { container } = list(500);
        // 96 px a card until measured: those that start before 500 + 800 px.
        expect(rows(container)).toHaveLength(14);
        expect(rows(container)[0]).toBe("t0");
        expect(spacers(container)).toEqual([`${(500 - 14) * 96}px`]);
    });

    it("moves the window with the scroll, keeping the list's length", () => {
        const { container } = list(500);
        scrollTop = 24_000;
        container.querySelector(".scroller")!.dispatchEvent(new Event("scroll"));
        const shown = rows(container);
        // The first card to end past 24000 - 800 px is the 242nd.
        expect(shown[0]).toBe("t241");
        expect(shown.length).toBeLessThan(40);
        const [above, below] = spacers(container).map((h) => parseFloat(h));
        expect(above).toBe(241 * 96);
        expect(above + shown.length * 96 + below).toBe(500 * 96);
    });
});

describe("WindowedList in a zoomed pane and in the lanes view", () => {
    afterEach(() => {
        cleanup();
        vi.restoreAllMocks();
        vi.unstubAllGlobals();
    });

    const items = (n: number) => Array.from({ length: n }, (_, i) => ({ id: `t${i}` }));
    const rows = (c: HTMLElement) => [...c.querySelectorAll(".row")].map((r) => r.textContent);

    it("reads the band in layout pixels when the pane is zoomed 1.5 times", () => {
        let scrollTop = 0;
        vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
            cb(0);
            return 0;
        });
        // Rectangles are in screen pixels (1.5 a layout pixel); offsetWidth, scrollTop and heights are layout pixels.
        vi.spyOn(HTMLElement.prototype, "offsetWidth", "get").mockImplementation(function (this: HTMLElement) {
            return this.classList.contains("kanban-window") ? 300 : 0;
        });
        vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
            const scroller = this.classList.contains("scroller");
            const top = scroller ? 0 : this.classList.contains("kanban-window") ? -scrollTop * 1.5 : 0;
            return { top, bottom: top + (scroller ? 500 * 1.5 : 0), left: 0, right: 0, width: this.classList.contains("kanban-window") ? 450 : 0, height: 0 } as DOMRect;
        });
        const { container } = render(() => (
            <div class="scroller" style={{ "overflow-y": "auto" }}>
                <WindowedList items={items(500)} keyOf={(t) => t.id} each={(t) => <span class="row">{t.id}</span>} />
            </div>
        ));
        scrollTop = 24_000;
        container.querySelector(".scroller")!.dispatchEvent(new Event("scroll"));
        // The same cards as the unzoomed list at 24000 px: the card ending past 24000 - 800 px is the 242nd.
        expect(rows(container)[0]).toBe("t241");
        expect(rows(container).length).toBeLessThan(40);
    });

    it("uses the ancestor that overflows, not a nearer one that grows to its content", () => {
        vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
            cb(0);
            return 0;
        });
        let scrollTop = 0;
        // The board (inner) has room for all of it; the lanes (outer) scroll.
        vi.spyOn(Element.prototype, "scrollHeight", "get").mockImplementation(function (this: Element) {
            return this.classList.contains("lanes") ? 50_000 : 100;
        });
        vi.spyOn(Element.prototype, "clientHeight", "get").mockImplementation(function (this: Element) {
            return this.classList.contains("lanes") ? 500 : 100;
        });
        vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
            const lanes = this.classList.contains("lanes");
            const top = lanes ? 0 : this.classList.contains("kanban-window") ? -scrollTop : 0;
            return { top, bottom: top + (lanes ? 500 : 0), left: 0, right: 0, width: 0, height: 0 } as DOMRect;
        });
        const { container } = render(() => (
            <div class="lanes" style={{ "overflow-y": "auto" }}>
                <div class="board" style={{ "overflow-y": "auto" }}>
                    <WindowedList items={items(500)} keyOf={(t) => t.id} each={(t) => <span class="row">{t.id}</span>} />
                </div>
            </div>
        ));
        // The first screen draws 14 cards, as it does in the other lists, not all 500.
        expect(rows(container)).toHaveLength(14);
        scrollTop = 24_000;
        container.querySelector(".lanes")!.dispatchEvent(new Event("scroll"));
        expect(rows(container)[0]).toBe("t241");
    });
});
