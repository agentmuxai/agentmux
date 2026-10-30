// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The transcript's zoom anchor: the text under the pointer is found, and after the zoom the scroll
 * moves by how far that text moved. jsdom has no layout, so the rectangles come from stubs the tests
 * move the way a zoom would.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { listZoomAnchor } from "./zoom-anchor-adapter";

const rect = (top: number, height: number, left = 0, width = 600): DOMRect =>
    ({ top, bottom: top + height, left, right: left + width, width, height, x: left, y: top }) as DOMRect;

let scroller: HTMLElement;
let row: HTMLElement;
let text: Text;
let glyphTop: number;
let zoom: number;
let following: boolean;
let rowStarts: Record<string, number>;
let caretAt: ReturnType<typeof vi.fn>;

const adapter = () =>
    listZoomAnchor({
        scroller: () => scroller,
        zoom: () => zoom,
        following: () => following,
        rowStart: (id) => rowStarts[id],
    });

beforeEach(() => {
    document.body.innerHTML = `<div id="s"><div data-node-id="n1"><p>hello world</p></div></div>`;
    scroller = document.getElementById("s")!;
    row = scroller.querySelector("[data-node-id]") as HTMLElement;
    text = row.querySelector("p")!.firstChild as Text;
    glyphTop = 190;
    zoom = 1;
    following = false;
    rowStarts = { n1: 1000 };
    scroller.getBoundingClientRect = () => rect(100, 400);
    row.getBoundingClientRect = () => rect(150, 120);
    // One 20 px glyph, wherever the test has moved it.
    Range.prototype.getClientRects = (() => [rect(glyphTop, 20, 40, 8)]) as unknown as Range["getClientRects"];
    Range.prototype.getBoundingClientRect = (() => rect(glyphTop, 20, 40, 8)) as unknown as Range["getBoundingClientRect"];
    caretAt = vi.fn(() => ({ offsetNode: text, offset: 3 }));
    (document as unknown as { caretPositionFromPoint: unknown }).caretPositionFromPoint = caretAt;
    document.elementFromPoint = () => row;
});

afterEach(() => {
    delete (document as unknown as { caretPositionFromPoint?: unknown }).caretPositionFromPoint;
    document.body.innerHTML = "";
});

describe("listZoomAnchor", () => {
    it("holds the text under the pointer: moves the scroll by how far the text moved, in unzoomed px", () => {
        const a = adapter();
        const token = a.capture({ clientX: 50, clientY: 200 })!;
        expect(caretAt).toHaveBeenCalledWith(50, 200);

        // The zoom doubles the text and the reflow carries the glyph 70 px (zoomed) below the pointer.
        zoom = 2;
        glyphTop = 260; // the glyph's middle was at the pointer (190 + 10 = 200); now 270 vs 200
        scroller.scrollTop = 0;
        expect(a.restore(token)).toBe(false);
        expect(scroller.scrollTop).toBe(35); // 70 zoomed px = 35 unzoomed

        glyphTop = 190; // that scroll put it back
        expect(a.restore(token)).toBe(true);
        expect(scroller.scrollTop).toBe(35); // and a settled position is left alone
    });

    it("leaves a position another writer already corrected alone: it measures, it does not replay", () => {
        const a = adapter();
        const token = a.capture({ clientX: 50, clientY: 200 })!;
        glyphTop = 190; // a live arrival re-anchored the rows before the pass
        scroller.scrollTop = 500;
        expect(a.restore(token)).toBe(true);
        expect(scroller.scrollTop).toBe(500);
    });

    it("anchors on the pane's centre when the point is outside the pane, or there is none", () => {
        const a = adapter();
        a.capture({ clientX: 50, clientY: 900 });
        expect(caretAt).toHaveBeenLastCalledWith(300, 300); // the scroller is (0,100) to (600,500)
        a.capture(null);
        expect(caretAt).toHaveBeenLastCalledWith(300, 300);
    });

    it("holds nothing for a pane that follows its output", () => {
        following = true;
        expect(adapter().capture({ clientX: 50, clientY: 200 })).toBeNull();
    });

    it("stops holding when the pane starts following", () => {
        const a = adapter();
        const token = a.capture({ clientX: 50, clientY: 200 })!;
        following = true;
        glyphTop = 400;
        scroller.scrollTop = 7;
        expect(a.restore(token)).toBe(true);
        expect(scroller.scrollTop).toBe(7);
    });

    it("holds nothing in a pane with no size", () => {
        scroller.getBoundingClientRect = () => rect(0, 0);
        expect(adapter().capture({ clientX: 50, clientY: 200 })).toBeNull();
    });

    it("falls back to the row when the pointer is over no text", () => {
        caretAt.mockReturnValue(null);
        const a = adapter();
        const token = a.capture({ clientX: 50, clientY: 200 });
        expect(token).toMatchObject({ nodeId: "n1", node: null });
    });

    it("does not trust text the engine found far from the point (a spacer, a gap): the row holds instead", () => {
        glyphTop = -600; // the nearest text it returned is a screen above the point
        const token = adapter().capture({ clientX: 50, clientY: 200 });
        expect(token).toMatchObject({ nodeId: "n1", node: null });
    });

    it("does not trust text that is in no row", () => {
        const stray = document.createElement("span");
        stray.textContent = "stray";
        scroller.appendChild(stray);
        caretAt.mockReturnValue({ offsetNode: stray.firstChild, offset: 1 });
        const token = adapter().capture({ clientX: 50, clientY: 200 });
        expect(token).toMatchObject({ nodeId: "n1", node: null });
    });

    it("falls back to the nearest row when the pointer is in a gap", () => {
        caretAt.mockReturnValue(null);
        document.elementFromPoint = () => scroller;
        const token = adapter().capture({ clientX: 50, clientY: 200 });
        expect(token).toMatchObject({ nodeId: "n1" });
    });

    it("uses the row's start from the layout slice while the text's row is not mounted", () => {
        const a = adapter();
        const token = a.capture({ clientX: 50, clientY: 200 })!; // 50 px below the row's top (150), viewport y 100
        row.remove(); // virtualized away
        zoom = 2;
        scroller.scrollTop = 0;

        expect(a.restore(token)).toBe(false);
        expect(scroller.scrollTop).toBe(1000 + 50 - 100 / 2); // row start + offset - viewportY (unzoomed)
    });

    it("has nothing to do when the row is unmounted and the slice does not know it", () => {
        const a = adapter();
        const token = a.capture({ clientX: 50, clientY: 200 })!;
        row.remove();
        rowStarts = {};
        scroller.scrollTop = 3;
        expect(a.restore(token)).toBe(true);
        expect(scroller.scrollTop).toBe(3);
    });
});
