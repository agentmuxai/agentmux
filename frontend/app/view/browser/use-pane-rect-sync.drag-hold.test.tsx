// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { holdRectDuringDrag } from "./use-pane-rect-sync";

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
