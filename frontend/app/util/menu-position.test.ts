// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Unit tests for the Phase 1 menu positioning primitive
// (SPEC_MENU_PAINTABLE_AREA_GUARD_2026_05_20 §9).
//
// jsdom returns all-zero rects from getBoundingClientRect, so every test
// stubs the geometry it needs explicitly: window.innerWidth/Height for the
// viewport, the floating menu's rect for its natural size, and per-element
// rects for synthetic native panes.

import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
    computeMenuPosition,
    getNativePaneRects,
    getPaintableArea,
} from "./menu-position";

const VIEWPORT_W = 1200;
const VIEWPORT_H = 800;

function makeRect(x: number, y: number, w: number, h: number): DOMRect {
    return {
        x,
        y,
        width: w,
        height: h,
        top: y,
        left: x,
        right: x + w,
        bottom: y + h,
        toJSON: () => ({ x, y, width: w, height: h }),
    } as DOMRect;
}

/**
 * A detached element with a fixed size.
 *
 * floating-ui sizes the floating element via `offsetWidth`/`offsetHeight`
 * (its `getCssDimensions`), not `getBoundingClientRect` — jsdom reports 0 for
 * both unless we define them. Stub all three so floating-ui sees a real menu.
 */
function elementWithRect(rect: DOMRect): HTMLElement {
    const el = document.createElement("div");
    el.getBoundingClientRect = () => rect;
    Object.defineProperty(el, "offsetWidth", { value: rect.width, configurable: true });
    Object.defineProperty(el, "offsetHeight", { value: rect.height, configurable: true });
    return el;
}

/** A `.browser-placeholder` mounted in the document with a stubbed rect. */
function mountNativePane(rect: DOMRect): HTMLElement {
    const el = document.createElement("div");
    el.className = "browser-placeholder";
    el.getBoundingClientRect = () => rect;
    document.body.appendChild(el);
    return el;
}

beforeEach(() => {
    window.innerWidth = VIEWPORT_W;
    window.innerHeight = VIEWPORT_H;
    // floating-ui's overflow detection intersects the boundary with the
    // 'viewport' rootBoundary, which it derives from
    // documentElement.clientWidth/Height (and visualViewport). jsdom reports
    // 0 for those, collapsing the effective clip rect to 0x0. Stub them so
    // floating-ui sees a real viewport — this mirrors a real browser.
    Object.defineProperty(document.documentElement, "clientWidth", {
        value: VIEWPORT_W,
        configurable: true,
    });
    Object.defineProperty(document.documentElement, "clientHeight", {
        value: VIEWPORT_H,
        configurable: true,
    });
});

afterEach(() => {
    document.body.innerHTML = "";
});

describe("getNativePaneRects", () => {
    it("returns no rects when there are no browser panes", () => {
        expect(getNativePaneRects()).toEqual([]);
    });

    it("returns the rect of each mounted browser-placeholder", () => {
        mountNativePane(makeRect(100, 100, 400, 300));
        mountNativePane(makeRect(600, 100, 400, 300));
        const rects = getNativePaneRects();
        expect(rects).toHaveLength(2);
        expect(rects[0].width).toBe(400);
    });

    it("skips zero-area placeholders (pane not yet created)", () => {
        mountNativePane(makeRect(0, 0, 0, 0));
        mountNativePane(makeRect(10, 10, 200, 200));
        expect(getNativePaneRects()).toHaveLength(1);
    });
});

describe("getPaintableArea", () => {
    it("is the whole viewport when there are no native panes", () => {
        const { largestFreeRect, rects } = getPaintableArea();
        expect(rects).toEqual([]);
        expect(largestFreeRect.width).toBe(VIEWPORT_W);
        expect(largestFreeRect.height).toBe(VIEWPORT_H);
    });

    it("excludes a pane docked against the right edge", () => {
        // Pane covers the right 400px; largest free rect is the left strip.
        mountNativePane(makeRect(800, 0, 400, VIEWPORT_H));
        const { largestFreeRect } = getPaintableArea();
        expect(largestFreeRect.left).toBe(0);
        expect(largestFreeRect.right).toBe(800);
        expect(largestFreeRect.height).toBe(VIEWPORT_H);
    });

    it("picks the larger free strip when a pane splits the viewport", () => {
        // Pane occupies the bottom 200px → top strip (1200x600) is largest.
        mountNativePane(makeRect(0, 600, VIEWPORT_W, 200));
        const { largestFreeRect } = getPaintableArea();
        expect(largestFreeRect.top).toBe(0);
        expect(largestFreeRect.bottom).toBe(600);
    });
});

describe("computeMenuPosition — window edges (§9)", () => {
    // 200x300 menu; floating-ui reads its natural size off the element rect.
    const menuRect = makeRect(0, 0, 200, 300);

    function menuEl(): HTMLElement {
        return elementWithRect(menuRect);
    }

    it("keeps the menu in-bounds when anchored near the top-left corner", async () => {
        const res = await computeMenuPosition(
            { anchor: { x: 5, y: 5 }, avoidNativePanes: false },
            menuEl(),
        );
        const left = parseInt(res.style.left as string, 10);
        const top = parseInt(res.style.top as string, 10);
        expect(left).toBeGreaterThanOrEqual(0);
        expect(top).toBeGreaterThanOrEqual(0);
    });

    it("shifts the menu back inside when anchored near the right edge", async () => {
        const res = await computeMenuPosition(
            { anchor: { x: VIEWPORT_W - 10, y: 100 }, avoidNativePanes: false },
            menuEl(),
        );
        const left = parseInt(res.style.left as string, 10);
        // Right edge of the 200px-wide menu must not pass the viewport.
        expect(left + 200).toBeLessThanOrEqual(VIEWPORT_W);
    });

    it("flips/shifts the menu up when anchored near the bottom edge", async () => {
        const res = await computeMenuPosition(
            { anchor: { x: 100, y: VIEWPORT_H - 10 }, avoidNativePanes: false },
            menuEl(),
        );
        const top = parseInt(res.style.top as string, 10);
        expect(top + 300).toBeLessThanOrEqual(VIEWPORT_H);
    });

    it("keeps the menu in-bounds anchored near the bottom-right corner", async () => {
        const res = await computeMenuPosition(
            {
                anchor: { x: VIEWPORT_W - 10, y: VIEWPORT_H - 10 },
                avoidNativePanes: false,
            },
            menuEl(),
        );
        const left = parseInt(res.style.left as string, 10);
        const top = parseInt(res.style.top as string, 10);
        expect(left + 200).toBeLessThanOrEqual(VIEWPORT_W);
        expect(top + 300).toBeLessThanOrEqual(VIEWPORT_H);
    });

    it("emits a position:fixed style", async () => {
        const res = await computeMenuPosition(
            { anchor: { x: 100, y: 100 }, avoidNativePanes: false },
            menuEl(),
        );
        expect(res.style.position).toBe("fixed");
    });
});

describe("computeMenuPosition — native pane avoidance (§9)", () => {
    it("offsets the menu away from a synthetic native-pane rect", async () => {
        // Pane covers the right half of the viewport. A menu anchored just
        // left of the pane edge must land entirely inside the left free
        // strip (right edge <= 600), not behind the pane.
        mountNativePane(makeRect(600, 0, 600, VIEWPORT_H));
        const menu = elementWithRect(makeRect(0, 0, 200, 300));

        const res = await computeMenuPosition(
            { anchor: { x: 580, y: 100 }, avoidNativePanes: true },
            menu,
        );
        const left = parseInt(res.style.left as string, 10);
        expect(left + 200).toBeLessThanOrEqual(600);
    });

    it("ignores native panes when avoidNativePanes is false", async () => {
        // Same pane, but avoidance off — the menu may extend past x=600 since
        // only the viewport clamps it. It must still stay within the viewport.
        mountNativePane(makeRect(600, 0, 600, VIEWPORT_H));
        const menu = elementWithRect(makeRect(0, 0, 200, 300));

        const res = await computeMenuPosition(
            { anchor: { x: 580, y: 100 }, avoidNativePanes: false },
            menu,
        );
        const left = parseInt(res.style.left as string, 10);
        // Allowed to overlap the pane region; only the viewport bounds it.
        expect(left + 200).toBeLessThanOrEqual(VIEWPORT_W);
        expect(left + 200).toBeGreaterThan(600);
    });
});

describe("computeMenuPosition — too-tall menu shrink (§5.2 step 1)", () => {
    it("clamps maxHeight below the menu's natural height", async () => {
        // A 200x2000 menu cannot fit in an 800px viewport — size() must emit
        // a maxHeight well under 2000 so the menu scrolls internally.
        const tallMenu = elementWithRect(makeRect(0, 0, 200, 2000));
        const res = await computeMenuPosition(
            { anchor: { x: 100, y: 400 }, avoidNativePanes: false },
            tallMenu,
        );
        expect(res.maxHeight).toBeLessThan(2000);
        expect(res.maxHeight).toBeLessThanOrEqual(VIEWPORT_H);
        expect(res.maxHeight).toBeGreaterThan(0);
    });

    it("clamps maxHeight further when a native pane shrinks the free area", async () => {
        // Pane covers the bottom half → free strip is only 400px tall.
        mountNativePane(makeRect(0, 400, VIEWPORT_W, 400));
        const tallMenu = elementWithRect(makeRect(0, 0, 200, 2000));
        const res = await computeMenuPosition(
            { anchor: { x: 100, y: 50 }, avoidNativePanes: true },
            tallMenu,
        );
        expect(res.maxHeight).toBeLessThanOrEqual(400);
    });
});
