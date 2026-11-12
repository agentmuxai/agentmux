// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { paneBodyBox, zoomIndicatorCenter } from "./zoom-indicator-position";

const WINDOW = { width: 1600, height: 900 };
const AMOUNT = { width: 200, height: 90 };

describe("zoomIndicatorCenter", () => {
    it("is the centre of the pane being zoomed, a left one and a right one", () => {
        expect(zoomIndicatorCenter({ x: 0, y: 40, width: 800, height: 800 }, AMOUNT, WINDOW)).toEqual({ x: 400, y: 440 });
        expect(zoomIndicatorCenter({ x: 800, y: 40, width: 800, height: 800 }, AMOUNT, WINDOW)).toEqual({ x: 1200, y: 440 });
    });

    it("keeps the whole amount inside the window when the pane is smaller than it, as near the pane's centre as fits", () => {
        // A pane 60 px wide in the window's top-left corner.
        expect(zoomIndicatorCenter({ x: 0, y: 0, width: 60, height: 40 }, AMOUNT, WINDOW)).toEqual({ x: 100, y: 45 });
        // And in the bottom-right corner.
        expect(zoomIndicatorCenter({ x: 1540, y: 860, width: 60, height: 40 }, AMOUNT, WINDOW)).toEqual({ x: 1500, y: 855 });
    });

    it("counts a pane partly outside the window by the part that is inside", () => {
        expect(zoomIndicatorCenter({ x: -400, y: 0, width: 1000, height: 900 }, AMOUNT, WINDOW)).toEqual({ x: 300, y: 450 });
        expect(zoomIndicatorCenter({ x: 1000, y: 0, width: 1000, height: 900 }, AMOUNT, WINDOW)).toEqual({ x: 1300, y: 450 });
    });

    it("is the window's centre with no pane or a pane that is off screen, and across the window when the amount is wider than it", () => {
        expect(zoomIndicatorCenter(null, AMOUNT, WINDOW)).toBeNull();
        expect(zoomIndicatorCenter({ x: 2000, y: 0, width: 500, height: 500 }, AMOUNT, WINDOW)).toBeNull();
        expect(zoomIndicatorCenter({ x: 0, y: 0, width: 100, height: 100 }, { width: 2000, height: 90 }, WINDOW)).toEqual({ x: 800, y: 50 });
    });
});

describe("paneBodyBox", () => {
    const rect = (x: number, y: number, width: number, height: number) => () =>
        ({ x, y, width, height, left: x, top: y, right: x + width, bottom: y + height, toJSON() {} }) as DOMRect;

    it("is the content area of the biggest element with the block's id, not a small part of it or a hidden copy", () => {
        document.body.innerHTML = `
            <div id="copy" data-blockid="b1"></div>
            <div id="frame" data-blockid="b1"><div id="header" data-blockid="b1"></div><div class="block-content" id="content"></div></div>`;
        const byId = (id: string) => document.getElementById(id) as HTMLElement;
        byId("copy").getBoundingClientRect = rect(0, 0, 0, 0);
        byId("frame").getBoundingClientRect = rect(100, 50, 600, 400);
        byId("header").getBoundingClientRect = rect(100, 50, 600, 30);
        byId("content").getBoundingClientRect = rect(100, 80, 600, 370);
        expect(paneBodyBox("b1")).toEqual({ x: 100, y: 80, width: 600, height: 370 });
    });

    it("ignores a bigger copy of the id parked outside the window (a small pane in a small window)", () => {
        document.body.innerHTML = `
            <div id="parked" data-blockid="b3"></div>
            <div id="frame" data-blockid="b3"><div class="block-content" id="content"></div></div>`;
        const byId = (id: string) => document.getElementById(id) as HTMLElement;
        byId("parked").getBoundingClientRect = rect(0, 10033, 127, 274);
        byId("frame").getBoundingClientRect = rect(0, 40, 122, 270);
        byId("content").getBoundingClientRect = rect(0, 74, 122, 236);
        expect(paneBodyBox("b3", document, { width: 380, height: 330 })).toEqual({ x: 0, y: 74, width: 122, height: 236 });
    });

    it("is null for a pane with no box here", () => {
        document.body.innerHTML = `<div id="frame" data-blockid="b2"></div>`;
        (document.getElementById("frame") as HTMLElement).getBoundingClientRect = rect(0, 0, 0, 0);
        expect(paneBodyBox("b2")).toBeNull();
        expect(paneBodyBox("nobody")).toBeNull();
    });
});
