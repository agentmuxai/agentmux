// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { createDragHold, holdRectDuringDrag } from "./use-pane-rect-sync";

describe("createDragHold", () => {
    it("takes the starting size at the drag's first rect, holds it for the drag, and drops it on release", () => {
        let dragging = false;
        const hold = createDragHold(() => dragging);
        const at = (w: number, h: number) => ({ x: 5, y: 6, width: w, height: h });

        expect(hold.apply(at(900, 700))).toEqual(at(900, 700));
        dragging = true;
        expect(hold.apply(at(800, 600))).toEqual(at(800, 600));
        expect(hold.apply(at(950, 650))).toEqual(at(800, 600));
        expect(hold.apply(at(500, 650))).toEqual(at(500, 600));
        expect(hold.sizeCss({ width: 950, height: 650 }, 1)).toEqual({ w: 800, h: 600 });
        dragging = false;
        expect(hold.apply(at(950, 650))).toEqual(at(950, 650));
        expect(hold.sizeCss({ width: 950, height: 650 }, 1)).toEqual({ w: 950, h: 650 });
        // The next drag starts from the size the pane has then.
        dragging = true;
        expect(hold.apply(at(950, 650))).toEqual(at(950, 650));
        expect(hold.apply(at(1000, 700))).toEqual(at(950, 650));
    });
});

// A native page is a window above the DOM: while a splitter is dragged it never grows past the pane's
// starting size (it lays out once, on release), and never stays larger than a pane that shrank.
describe("holdRectDuringDrag", () => {
    const start = { width: 800, height: 600 };

    it("leaves the rect as it is outside a drag", () => {
        const rect = { x: 10, y: 40, width: 900, height: 700 };
        expect(holdRectDuringDrag(rect, null)).toBe(rect);
    });

    it("keeps the starting size while the pane grows, at the pane's own place", () => {
        expect(holdRectDuringDrag({ x: 10, y: 40, width: 900, height: 650 }, start)).toEqual({
            x: 10,
            y: 40,
            width: 800,
            height: 600,
        });
    });

    it("follows a pane that shrinks, so the page never covers the next pane", () => {
        expect(holdRectDuringDrag({ x: 10, y: 40, width: 500, height: 450 }, start)).toEqual({
            x: 10,
            y: 40,
            width: 500,
            height: 450,
        });
    });

    it("holds each axis on its own", () => {
        expect(holdRectDuringDrag({ x: 0, y: 0, width: 900, height: 300 }, start)).toEqual({
            x: 0,
            y: 0,
            width: 800,
            height: 300,
        });
    });

    it("passes a collapsed rect through: 0x0 means hidden to the host", () => {
        const hidden = { x: 0, y: 0, width: 0, height: 0 };
        expect(holdRectDuringDrag(hidden, start)).toBe(hidden);
    });
});
