// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

import { describe, expect, it } from "vitest";
import {
    FLOATER_BROWSER_EDGE_INSET,
    FLOATER_CORNER_RESIZE_SIZE,
    FLOATER_EDGE_BOTTOM,
    FLOATER_EDGE_BOTTOM_LEFT,
    FLOATER_EDGE_BOTTOM_RIGHT,
    FLOATER_EDGE_LEFT,
    FLOATER_EDGE_NONE,
    FLOATER_EDGE_RESIZE_BORDER,
    FLOATER_EDGE_RIGHT,
    FLOATER_EDGE_TOP,
    FLOATER_EDGE_TOP_LEFT,
    FLOATER_EDGE_TOP_RIGHT,
    FLOATER_RESIZE_CURSORS,
    FLOATER_TOP_CORNER_DEPTH,
    FLOATER_TOP_RESIZE_BORDER,
    floaterEdgeDirection,
    isFloaterBandControl,
} from "./floater-resize";

const W = 800;
const H = 600;
const dir = (x: number, y: number) => floaterEdgeDirection(x, y, W, H);

describe("the floater resize band", () => {
    it("is at least 12 CSS px on the left, right and bottom, and larger than the 8px that was too thin to find", () => {
        expect(FLOATER_EDGE_RESIZE_BORDER).toBeGreaterThanOrEqual(12);
        expect(dir(FLOATER_EDGE_RESIZE_BORDER, H / 2)).toBe(FLOATER_EDGE_LEFT);
        expect(dir(FLOATER_EDGE_RESIZE_BORDER + 1, H / 2)).toBe(FLOATER_EDGE_NONE);
        expect(dir(W - FLOATER_EDGE_RESIZE_BORDER, H / 2)).toBe(FLOATER_EDGE_RIGHT);
        expect(dir(W - FLOATER_EDGE_RESIZE_BORDER - 1, H / 2)).toBe(FLOATER_EDGE_NONE);
        expect(dir(W / 2, H - FLOATER_EDGE_RESIZE_BORDER)).toBe(FLOATER_EDGE_BOTTOM);
        expect(dir(W / 2, H - FLOATER_EDGE_RESIZE_BORDER - 1)).toBe(FLOATER_EDGE_NONE);
    });

    it("keeps the top band shallow so the header buttons below it stay clickable", () => {
        expect(dir(W / 2, FLOATER_TOP_RESIZE_BORDER)).toBe(FLOATER_EDGE_TOP);
        expect(dir(W / 2, FLOATER_TOP_RESIZE_BORDER + 1)).toBe(FLOATER_EDGE_NONE);
        expect(FLOATER_TOP_RESIZE_BORDER).toBeLessThanOrEqual(FLOATER_EDGE_RESIZE_BORDER);
    });

    it("makes each corner a square reaching further along the edges than the edge band is deep", () => {
        const c = FLOATER_CORNER_RESIZE_SIZE;
        expect(c).toBeGreaterThan(FLOATER_EDGE_RESIZE_BORDER);
        // Bottom corners reach c on both axes.
        expect(dir(c, H - c)).toBe(FLOATER_EDGE_BOTTOM_LEFT);
        expect(dir(W - c, H - c)).toBe(FLOATER_EDGE_BOTTOM_RIGHT);
        expect(dir(c + 1, H - c)).toBe(FLOATER_EDGE_NONE);
        expect(dir(c, H - c - 1)).toBe(FLOATER_EDGE_NONE);
        // Top corners reach c along the top edge but only the top-corner depth down.
        expect(dir(c, FLOATER_TOP_CORNER_DEPTH)).toBe(FLOATER_EDGE_TOP_LEFT);
        expect(dir(W - c, FLOATER_TOP_CORNER_DEPTH)).toBe(FLOATER_EDGE_TOP_RIGHT);
        expect(dir(c, FLOATER_TOP_CORNER_DEPTH + 1)).toBe(FLOATER_EDGE_NONE);
    });

    it("reads a point near an edge and a corner as the corner", () => {
        expect(dir(0, 0)).toBe(FLOATER_EDGE_TOP_LEFT);
        expect(dir(W, 0)).toBe(FLOATER_EDGE_TOP_RIGHT);
        expect(dir(0, H)).toBe(FLOATER_EDGE_BOTTOM_LEFT);
        expect(dir(W, H)).toBe(FLOATER_EDGE_BOTTOM_RIGHT);
        expect(dir(5, H - 15)).toBe(FLOATER_EDGE_BOTTOM_LEFT);
        // Along the left edge, past the corner, it is the plain edge.
        expect(dir(5, H - FLOATER_CORNER_RESIZE_SIZE - 1)).toBe(FLOATER_EDGE_LEFT);
    });

    it("finds no band in the middle of the window", () => {
        expect(dir(W / 2, H / 2)).toBe(FLOATER_EDGE_NONE);
        expect(dir(W / 2, 100)).toBe(FLOATER_EDGE_NONE);
    });

    it("has a cursor for every direction, a diagonal one for each corner", () => {
        expect(FLOATER_RESIZE_CURSORS[FLOATER_EDGE_NONE]).toBe("");
        expect(FLOATER_RESIZE_CURSORS[FLOATER_EDGE_LEFT]).toBe("ew-resize");
        expect(FLOATER_RESIZE_CURSORS[FLOATER_EDGE_RIGHT]).toBe("ew-resize");
        expect(FLOATER_RESIZE_CURSORS[FLOATER_EDGE_TOP]).toBe("ns-resize");
        expect(FLOATER_RESIZE_CURSORS[FLOATER_EDGE_BOTTOM]).toBe("ns-resize");
        expect(FLOATER_RESIZE_CURSORS[FLOATER_EDGE_TOP_LEFT]).toBe("nwse-resize");
        expect(FLOATER_RESIZE_CURSORS[FLOATER_EDGE_BOTTOM_RIGHT]).toBe("nwse-resize");
        expect(FLOATER_RESIZE_CURSORS[FLOATER_EDGE_TOP_RIGHT]).toBe("nesw-resize");
        expect(FLOATER_RESIZE_CURSORS[FLOATER_EDGE_BOTTOM_LEFT]).toBe("nesw-resize");
    });

    it("never insets a browser pane's web content deeper than the band, so the band stays reachable", () => {
        expect(FLOATER_BROWSER_EDGE_INSET).toBeLessThanOrEqual(FLOATER_EDGE_RESIZE_BORDER);
    });

    it("lets a pane header's button win over the band: a press or hover on it, or on what is inside it, is not a resize", () => {
        document.body.innerHTML =
            '<div data-role="block-header" id="header"><button id="close"><svg id="glyph"></svg></button><span id="gap"></span><div role="button" id="role"></div></div>';
        const el = (id: string) => document.getElementById(id);
        expect(isFloaterBandControl(el("close"))).toBe(true);
        expect(isFloaterBandControl(el("glyph"))).toBe(true);
        expect(isFloaterBandControl(el("role"))).toBe(true);
        expect(isFloaterBandControl(el("gap"))).toBe(false);
        expect(isFloaterBandControl(el("header"))).toBe(false);
        expect(isFloaterBandControl(null)).toBe(false);
    });

    it("does not let a button in the pane's content win: a sidebar button at the window's edge keeps the band", () => {
        // A floating Hangar's Places list starts at x 0; a toolbar can end at the right or bottom edge.
        document.body.innerHTML =
            '<div data-role="block-header"><button id="close"></button></div><div class="content"><nav><button id="places"><span id="label"></span></button></nav><div role="button" id="tool"></div><footer><button id="foot"></button></footer></div>';
        const el = (id: string) => document.getElementById(id);
        for (const id of ["places", "label", "tool", "foot"]) expect(isFloaterBandControl(el(id))).toBe(false);
        expect(isFloaterBandControl(el("close"))).toBe(true);
    });
});
