// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * The file tree's zoom anchor: the row under the pointer stays under it through a run of zoom steps. A fake
 * tree stands in for the DOM's layout: row rects are zoomed px (`index * row height * zoom`, less the
 * scroll), `scrollTop` is unzoomed and the engine snaps it to whole px, which is what leaves a step a
 * fraction of a px off (SPEC_ZOOM_ANCHOR_AND_AUDIT_2026_10_29.md §4.1).
 */

import { beforeEach, describe, expect, it } from "vitest";
import { TOLERANCE_PX } from "@/app/store/zoom-anchor-dom";
import { treeZoomAnchor } from "./editor-tree-zoom-anchor";

const BOX_TOP = 100;
const ROW_PX = 19; // a row's height at 100%
const POINTER = { clientX: 50, clientY: 240 };

let zoom: number;
let scrollTop: number;
let scroller: HTMLElement;
/** Row `i` is at `BOX_TOP + (i * ROW_PX * zoom) - scrollTop * zoom`. */
const rows: HTMLElement[] = [];

function rowRect(i: number): DOMRect {
    const top = BOX_TOP + i * ROW_PX * zoom - scrollTop * zoom;
    return { top, bottom: top + ROW_PX * zoom, left: 0, right: 200, width: 200, height: ROW_PX * zoom, x: 0, y: top } as DOMRect;
}

/** The row at a height (the pointer's by default), as a person sees it. */
const rowAt = (y: number): number => rows.findIndex((_, i) => rowRect(i).top <= y && y < rowRect(i).bottom);
const rowUnderPointer = (): number => rowAt(POINTER.clientY);

beforeEach(() => {
    zoom = 1;
    scrollTop = 0;
    rows.length = 0;
    document.body.innerHTML = `<div id="s"></div>`;
    scroller = document.getElementById("s") as HTMLElement;
    scroller.getBoundingClientRect = () => ({ top: BOX_TOP, bottom: BOX_TOP + 400, left: 0, right: 200, width: 200, height: 400, x: 0, y: BOX_TOP }) as DOMRect; // perf:allow-layout-read — a stub that stands in for the scroller box
    // The engine snaps the scroll position to whole device px, which at a zoom is `1 / zoom` unzoomed px.
    Object.defineProperty(scroller, "scrollTop", {
        get: () => scrollTop,
        set: (v: number) => {
            scrollTop = Math.round(v * zoom) / zoom;
        },
        configurable: true,
    });
    for (let i = 0; i < 200; i++) {
        const el = document.createElement("div");
        el.setAttribute("role", "treeitem");
        el.dataset.path = `/r${i}`;
        el.getBoundingClientRect = () => rowRect(i); // perf:allow-layout-read — a stub that stands in for the row's box
        scroller.appendChild(el);
        rows.push(el);
    }
    // jsdom has no hit test: the row at the point's height.
    document.elementFromPoint = (_x: number, y: number) => rows[rowAt(y)] ?? null;
});

const adapter = () => treeZoomAnchor({ scroller: () => scroller, zoom: () => zoom });

/** One zoom step as the hook runs it: capture, change the zoom, restore each frame until in place (at most 10). */
function step(a: ReturnType<typeof adapter>, to: number, point = POINTER): void {
    const token = a.capture(point)!;
    zoom = to;
    for (let frame = 0; frame < 10 && !a.restore(token); frame++);
}

describe("treeZoomAnchor", () => {
    it("holds the row's offset from the scroller's top, within the tolerance, through a step", () => {
        scrollTop = 20;
        const a = adapter();
        const before = rowRect(rowUnderPointer()).top - BOX_TOP;
        const row = rowUnderPointer();
        step(a, 1.1);
        expect(Math.abs(rowRect(row).top - BOX_TOP - before)).toBeLessThan(TOLERANCE_PX);
    });

    it("keeps the row within the tolerance of where it started through six steps, 0.1 apart and each its own gesture", () => {
        scrollTop = 20;
        const a = adapter();
        const row = rowUnderPointer();
        const start = rowRect(row).top - BOX_TOP;
        for (let n = 1; n <= 6; n++) {
            step(a, 1 + n / 10);
            expect(rowUnderPointer()).toBe(row);
            expect(Math.abs(rowRect(row).top - BOX_TOP - start), `after step ${n}`).toBeLessThanOrEqual(TOLERANCE_PX + 1e-9);
        }
    });

    it("keeps the offset it first held when the next step comes after the last one stopped a fraction off", () => {
        scrollTop = 20;
        const a = adapter();
        const first = a.capture(POINTER)!;
        // The step settled 0.4 px off, inside the tolerance: the restore had nothing more to do.
        scrollTop = 20.4;
        expect(rowUnderPointer()).toBe(Number(first.path.slice(2)));
        const second = a.capture(POINTER)!;
        expect(second.offsetY).toBe(first.offsetY);
    });

    it("captures afresh when the pointer has moved to another row", () => {
        scrollTop = 20;
        const a = adapter();
        const first = a.capture(POINTER)!;
        const moved = { clientX: 50, clientY: POINTER.clientY + 60 };
        const second = a.capture(moved)!;
        expect(second.path).not.toBe(first.path);
    });

    it("captures the offset afresh when the reader scrolled the same row a few px away and the pointer stayed", () => {
        // Rows 28.5 px tall at 150%; the row under the pointer stays the same one after a 6 px (zoomed) scroll.
        zoom = 1.5;
        scrollTop = 10;
        const a = adapter();
        const first = a.capture(POINTER)!;
        scrollTop = 14;
        expect(rowUnderPointer()).toBe(Number(first.path.slice(2)));
        const second = a.capture(POINTER)!;
        expect(second.path).toBe(first.path);
        expect(second.offsetY).toBeCloseTo(first.offsetY - 6, 5);
    });

    it("has nothing to hold without a scroller", () => {
        const a = treeZoomAnchor({ scroller: () => null, zoom: () => 1 });
        expect(a.capture(POINTER)).toBeNull();
    });
});
