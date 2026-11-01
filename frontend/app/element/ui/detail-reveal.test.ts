// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * revealDetail brings a panel that opened under a card into view: just far enough that its bottom edge shows,
 * and never so far that the card leaves the top of the pane. jsdom has no layout, so the rects and sizes
 * are given.
 */

import { afterEach, describe, expect, it } from "vitest";
import { findScrollParent, revealDetail } from "./detail-reveal";

const rect = (top: number, bottom: number): DOMRect =>
    ({ top, bottom, left: 0, right: 100, width: 100, height: bottom - top, x: 0, y: top, toJSON: () => ({}) }) as DOMRect;

/** A scrolling pane (top..bottom on screen) holding a card and the panel under it. */
function build(opts: { pane: [number, number]; card: [number, number]; panel: [number, number]; offsetHeight?: number }) {
    const pane = document.createElement("div");
    pane.style.overflowY = "auto";
    Object.defineProperty(pane, "scrollHeight", { value: 5000, configurable: true });
    Object.defineProperty(pane, "clientHeight", { value: opts.pane[1] - opts.pane[0], configurable: true });
    Object.defineProperty(pane, "offsetHeight", { value: opts.offsetHeight ?? opts.pane[1] - opts.pane[0], configurable: true });
    pane.getBoundingClientRect = () => rect(...opts.pane);
    const card = document.createElement("div");
    card.getBoundingClientRect = () => rect(...opts.card);
    const panel = document.createElement("div");
    panel.getBoundingClientRect = () => rect(...opts.panel);
    pane.append(card, panel);
    document.body.append(pane);
    return { pane, card, panel };
}

afterEach(() => {
    document.body.innerHTML = "";
});

describe("revealDetail", () => {
    it("finds the nearest ancestor that scrolls", () => {
        const { pane, panel } = build({ pane: [0, 500], card: [10, 60], panel: [70, 200] });
        expect(findScrollParent(panel)).toBe(pane);
        expect(findScrollParent(undefined)).toBeNull();
    });

    it("leaves the pane alone when the panel's bottom is already in view", () => {
        const { pane, card, panel } = build({ pane: [0, 500], card: [10, 60], panel: [70, 480] });
        revealDetail(panel, card);
        expect(pane.scrollTop).toBe(0);
    });

    it("scrolls by exactly what hangs below the pane, so the panel's bottom is just in view", () => {
        const { pane, card, panel } = build({ pane: [0, 500], card: [300, 350], panel: [360, 540] });
        revealDetail(panel, card);
        expect(pane.scrollTop).toBe(40);
    });

    it("never scrolls the card out of the top of the pane", () => {
        const { pane, card, panel } = build({ pane: [0, 500], card: [30, 80], panel: [90, 900] });
        revealDetail(panel, card);
        // 400px hang below, but the card is only 30px from the top.
        expect(pane.scrollTop).toBe(30);
    });

    it("converts screen px to scroll px when the pane is zoomed", () => {
        // 500px on screen is 400 css px: a zoom of 1.25.
        const { pane, card, panel } = build({ pane: [0, 500], card: [300, 350], panel: [360, 550], offsetHeight: 400 });
        revealDetail(panel, card);
        expect(pane.scrollTop).toBe(40);
    });

    it("does nothing without a scrolling ancestor", () => {
        const panel = document.createElement("div");
        const card = document.createElement("div");
        document.body.append(card, panel);
        expect(() => revealDetail(panel, card)).not.toThrow();
    });
});
