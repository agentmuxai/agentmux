// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The Editor's zoom anchor: a document position is held, and each pass scrolls by how far the view says
 * that position moved. A fake view stands in for CodeMirror, its answers moved the way a zoom would.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";
import { editorZoomAnchor, type ZoomEditorView } from "./editor-zoom-anchor";

const rect = (top: number, height: number): DOMRect =>
    ({ top, bottom: top + height, left: 0, right: 600, width: 600, height, x: 0, y: top }) as DOMRect;

let scroller: HTMLElement;
let zoom: number;
/** Where the held position's line is on screen: its top and height, or null when it is not drawn. */
let drawn: { top: number; bottom: number } | null;
let block: { top: number; height: number };
let documentTop: number;
let posAtCoords: ReturnType<typeof vi.fn>;
let view: ZoomEditorView | null;

const adapter = () => editorZoomAnchor({ view: () => view, zoom: () => zoom });

beforeEach(() => {
    scroller = document.createElement("div");
    scroller.getBoundingClientRect = () => rect(100, 400);
    zoom = 1;
    drawn = { top: 190, bottom: 210 };
    block = { top: 1000, height: 20 };
    documentTop = -900;
    posAtCoords = vi.fn(() => 321);
    view = {
        scrollDOM: scroller,
        get documentTop() {
            return documentTop;
        },
        posAtCoords: posAtCoords as unknown as ZoomEditorView["posAtCoords"],
        coordsAtPos: () => drawn,
        lineBlockAt: () => block,
    };
});

describe("editorZoomAnchor", () => {
    it("holds the position under the pointer: scrolls by how far it moved, in unzoomed px", () => {
        const a = adapter();
        const token = a.capture({ clientX: 50, clientY: 200 })!; // the middle of the line at 190 to 210
        expect(posAtCoords).toHaveBeenCalledWith({ x: 50, y: 200 }, false);
        expect(token.pos).toBe(321);
        expect(token.glyphY).toBeCloseTo(0.5);

        zoom = 2;
        drawn = { top: 260, bottom: 300 }; // the line doubled and sits 70 px (zoomed) lower: its middle is at 280
        scroller.scrollTop = 0;
        expect(a.restore(token)).toBe(false);
        expect(scroller.scrollTop).toBe(40); // 80 zoomed px = 40 unzoomed

        drawn = { top: 180, bottom: 220 }; // that put it back (middle 200)
        expect(a.restore(token)).toBe(true);
        expect(scroller.scrollTop).toBe(40);
    });

    it("places a line that is not drawn by the view's height map", () => {
        const a = adapter();
        const token = a.capture({ clientX: 50, clientY: 200 })!;
        zoom = 2;
        drawn = null;
        documentTop = -1500; // the line's top is at -1500 + 1000 * 2 = 500, its middle at 520; the point was at y 200
        scroller.scrollTop = 0;
        expect(a.restore(token)).toBe(false);
        expect(scroller.scrollTop).toBe(160); // 320 zoomed px / 2
    });

    it("uses the scroller's centre when the pointer is outside it or unknown", () => {
        adapter().capture(null);
        expect(posAtCoords).toHaveBeenLastCalledWith({ x: 300, y: 300 }, false);
        adapter().capture({ clientX: 9000, clientY: 9000 });
        expect(posAtCoords).toHaveBeenLastCalledWith({ x: 300, y: 300 }, false);
    });

    it("holds nothing without a view or a layout, and has nothing to do when the position is gone", () => {
        const a = adapter();
        const token = a.capture({ clientX: 50, clientY: 200 })!;
        scroller.getBoundingClientRect = () => rect(0, 0);
        expect(a.capture(null)).toBeNull();
        scroller.getBoundingClientRect = () => rect(100, 400);

        view!.coordsAtPos = () => {
            throw new RangeError("Position out of range");
        };
        (view as { lineBlockAt: unknown }).lineBlockAt = () => {
            throw new RangeError("Position out of range");
        };
        expect(a.restore(token)).toBe(true);

        view = null;
        expect(a.capture(null)).toBeNull();
        expect(a.restore(token)).toBe(true);
    });
});
