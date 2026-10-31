// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The PDF viewer's zoom: the pane's level is a multiple of the width that fits, and the page under the
 * pointer is held across a zoom. Fake page boxes stand in for pdf.js's laid out pages.
 */

import { beforeEach, describe, expect, it } from "vitest";
import { pdfZoomAnchor, scaleFactorTo, scaleFor, zoomForScale } from "./pdf-zoom";

const box = (left: number, top: number, width: number, height: number): DOMRect =>
    ({ left, top, width, height, right: left + width, bottom: top + height, x: left, y: top }) as DOMRect;

let container: HTMLElement;
let chromeZoom: number;
/** Each page's box in client px, by page number, as pdf.js lays them out at the scale shown. */
let pages: Map<number, DOMRect>;

function layout(width: number, height: number, firstTop: number, gap = 10): void {
    pages = new Map();
    for (let n = 1; n <= 5; n++) pages.set(n, box(100, firstTop + (n - 1) * (height + gap), width, height));
}

beforeEach(() => {
    container = document.createElement("div");
    // perf:allow-layout-read — stubbed boxes, jsdom has no layout
    container.getBoundingClientRect = () => box(0, 100, 500, 800);
    chromeZoom = 1;
    document.body.replaceChildren(container);
    for (let n = 1; n <= 5; n++) {
        const page = document.createElement("div");
        page.className = "page";
        page.dataset.pageNumber = String(n);
        // perf:allow-layout-read — stubbed boxes, jsdom has no layout
        page.getBoundingClientRect = () => pages.get(n)!;
        container.appendChild(page);
    }
    layout(300, 400, 50);
});

describe("the pane's zoom as a pdf.js scale", () => {
    it("is a multiple of the width that fits", () => {
        expect(scaleFor(0.8, 1)).toBeCloseTo(0.8);
        expect(scaleFor(0.8, 1.5)).toBeCloseTo(1.2);
        expect(scaleFor(0.8, 0.5)).toBeCloseTo(0.4);
    });

    it("asks pdf.js for a ratio, and for none when the pages already are at that scale", () => {
        expect(scaleFactorTo(0.8, 1.2)).toBeCloseTo(1.5);
        expect(scaleFactorTo(0.8, 0.8)).toBeNull();
        expect(scaleFactorTo(0.8, 0.801)).toBeNull();
        expect(scaleFactorTo(0, 1)).toBeNull();
    });

    it("gives the pane zoom at which a fit scale shows", () => {
        expect(zoomForScale(0.6, 0.8)).toBeCloseTo(0.75);
        expect(zoomForScale(0.6, 0)).toBe(1);
    });
});

describe("pdfZoomAnchor", () => {
    const adapter = () => pdfZoomAnchor({ container: () => container, chromeZoom: () => chromeZoom });

    it("holds the point of the page under the pointer: scrolls by how far it moved", () => {
        const a = adapter();
        // Page 2 spans y 460 to 860 (top 50 + 410); the pointer is 25% across and 50% down it.
        const token = a.capture({ clientX: 175, clientY: 660 })!;
        expect(token).toMatchObject({ page: 2, fx: 0.25, fy: 0.5, viewportX: 175, viewportY: 560 });

        // Twice the scale: pages 600 x 800, so page 2 now starts at 50 + 810.
        layout(600, 800, 50);
        container.scrollLeft = 0;
        container.scrollTop = 0;
        expect(a.restore(token)).toBe(false);
        expect(container.scrollLeft).toBeCloseTo(100 + 0.25 * 600 - 175);
        expect(container.scrollTop).toBeCloseTo(50 + 810 + 0.5 * 800 - 660);
    });

    it("reports done when the point is where it was", () => {
        const a = adapter();
        const token = a.capture({ clientX: 175, clientY: 660 })!;
        expect(a.restore(token)).toBe(true);
    });

    it("divides the move by the chrome zoom: rects are zoomed px, scrollTop is not", () => {
        chromeZoom = 2;
        const a = adapter();
        const token = a.capture({ clientX: 175, clientY: 660 })!;
        layout(300, 400, 150); // every page 100 px lower
        container.scrollTop = 0;
        a.restore(token);
        expect(container.scrollTop).toBeCloseTo(50);
    });

    it("holds the container's centre when the pointer is elsewhere", () => {
        const token = adapter().capture({ clientX: 900, clientY: 1900 })!;
        expect(token.viewportX).toBe(250);
        expect(token.viewportY).toBe(400);
    });

    it("has nothing to hold without pages, or when the page it held is gone", () => {
        const a = adapter();
        const token = a.capture({ clientX: 175, clientY: 660 })!;
        container.replaceChildren();
        expect(a.restore(token)).toBe(true);
        expect(a.capture(null)).toBeNull();
    });
});
