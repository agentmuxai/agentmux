// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import { landingSiteOf, parseWash, siteUnderZone } from "./landingSite";

interface Box {
    left: number;
    top: number;
    width: number;
    height: number;
}

/** An element that reports `box` as its rect. */
function boxed(box: Box, cls = ""): HTMLElement {
    const el = document.createElement("div");
    el.className = cls;
    el.getBoundingClientRect = () => ({ ...box, x: box.left, y: box.top, right: box.left + box.width, bottom: box.top + box.height, toJSON: () => box }) as DOMRect;
    return el;
}

/** A landing zone drawn as tilelayout.scss draws it, unless `over` says otherwise. */
function zoneAt(box: Box, over: Partial<CSSStyleDeclaration> = {}): HTMLElement {
    const el = boxed(box, "placeholder");
    Object.assign(el.style, { backgroundColor: "rgba(65, 159, 224, 0.26)", border: "2px solid rgb(65, 159, 224)", opacity: "1" }, over);
    return el;
}

afterEach(() => {
    document.body.innerHTML = "";
    vi.restoreAllMocks();
});

describe("parseWash", () => {
    it("reads a computed rgb or rgba colour, and no other form", () => {
        expect(parseWash("rgba(65, 159, 224, 0.26)")).toEqual({ r: 65, g: 159, b: 224, a: 0.26 });
        expect(parseWash("rgb(10, 20, 30)")).toEqual({ r: 10, g: 20, b: 30, a: 1 });
        expect(parseWash("color(srgb 0.2 0.4 0.6 / 0.5)")).toBeUndefined();
        expect(parseWash("")).toBeUndefined();
    });
});

describe("siteUnderZone", () => {
    const pane = { left: 103, top: 53, width: 594, height: 394 };
    const zone = { left: 100, top: 50, width: 600, height: 400 };

    it("is the pane's own rect with nothing drawn over it when there is no zone", () => {
        expect(siteUnderZone(boxed(pane), null)).toEqual({ rect: pane, viewportWidth: window.innerWidth });
    });

    it("is the pane inside the zone's border, with the zone's fill as the wash to take out", () => {
        // The zone's inside starts 3 px in (its 2 px border and a px for rounding): exactly where the pane is.
        expect(siteUnderZone(boxed(pane), zoneAt(zone))).toEqual({ rect: pane, viewportWidth: window.innerWidth, wash: { r: 65, g: 159, b: 224, a: 0.26 } });
        // A pane that fills the zone to its edge is taken inside the border, where only the fill is over it.
        expect(siteUnderZone(boxed(zone), zoneAt(zone))?.rect).toEqual({ left: 103, top: 53, width: 594, height: 394 });
    });

    it("gives no site while the zone is still arriving, or is somewhere else", () => {
        expect(siteUnderZone(boxed(pane), zoneAt(zone, { opacity: "0.5" }))).toBeUndefined();
        expect(siteUnderZone(boxed(pane), zoneAt(zone, { transform: "matrix(0.92, 0, 0, 0.92, 0, 0)" }))).toBeUndefined();
        // A zone over another pane covers little of this one.
        expect(siteUnderZone(boxed(pane), zoneAt({ ...zone, left: 600 }))).toBeUndefined();
    });

    it("gives no site for a fill that cannot be taken back out of a picture", () => {
        // Opaque: nothing of the pane is in the picture.
        expect(siteUnderZone(boxed(pane), zoneAt(zone, { backgroundColor: "rgb(65, 159, 224)" }))).toBeUndefined();
    });

    it("has no wash when the zone's fill is clear", () => {
        expect(siteUnderZone(boxed(pane), zoneAt(zone, { backgroundColor: "rgba(65, 159, 224, 0)" }))).toEqual({ rect: pane, viewportWidth: window.innerWidth });
    });

    it("gives no site for a pane with no size", () => {
        expect(siteUnderZone(boxed({ left: 0, top: 0, width: 0, height: 0 }), null)).toBeUndefined();
    });
});

describe("landingSiteOf", () => {
    function layoutWith(pane: HTMLElement, zone?: HTMLElement) {
        const layout = document.createElement("div");
        const display = document.createElement("div");
        pane.setAttribute("data-blockid", "b1");
        display.append(pane);
        layout.append(display);
        if (zone) {
            const container = document.createElement("div");
            container.className = "placeholder-container";
            container.append(zone);
            layout.append(container);
        }
        document.body.append(layout);
        return display;
    }
    const pane = { left: 103, top: 53, width: 594, height: 394 };

    it("reads the pane under the layout's own landing zone, once the panes have come to rest", () => {
        const display = layoutWith(boxed(pane), zoneAt({ left: 100, top: 50, width: 600, height: 400 }));
        const settled = { left: 100, top: 50, width: 600, height: 400 };
        const model = { landingSettled: () => settled, displayContainerRef: { current: display } };
        expect(landingSiteOf(model, "b1")).toMatchObject({ rect: pane, wash: { r: 65, g: 159, b: 224, a: 0.26 } });
        // Another pane's id finds nothing.
        expect(landingSiteOf(model, "b2")).toBeUndefined();
    });

    it("reads nothing while the panes are still on their way", () => {
        const display = layoutWith(boxed(pane));
        expect(landingSiteOf({ landingSettled: () => undefined, displayContainerRef: { current: display } }, "b1")).toBeUndefined();
    });
});
