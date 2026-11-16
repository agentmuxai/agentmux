// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createMemo, createRoot } from "solid-js";
import { describe, expect, it } from "vitest";
import { useAtlasLasso } from "./atlas-lasso";
import { gapsAt } from "./atlas-layout";
import { useAtlasPanZoom, type AtlasPanZoom } from "./atlas-pan-zoom";
import { pointerScale } from "./atlas-viewport";

/** A pane the person zoomed to 125%: it is 800 px of its own and is drawn 1000 px wide. */
const ZOOMED = { left: 100, top: 50, width: 1000, height: 750 };
const OWN = { w: 800, h: 600 };

function pointer(clientX: number, clientY: number, type = "pointermove"): PointerEvent {
    return { pointerId: 1, button: 0, shiftKey: false, clientX, clientY, type } as PointerEvent;
}

function viewportAt(rect: typeof ZOOMED): HTMLElement {
    return {
        getBoundingClientRect: () => rect,
        clientWidth: OWN.w,
        clientHeight: OWN.h,
        setPointerCapture: () => {},
    } as unknown as HTMLElement;
}

describe("pointerScale", () => {
    it("is 1 in a pane at its normal zoom", () => {
        expect(pointerScale({ width: 800, height: 600 }, OWN)).toEqual({ sx: 1, sy: 1 });
    });

    it("is the pane zoom in a zoomed pane", () => {
        expect(pointerScale(ZOOMED, OWN)).toEqual({ sx: 1.25, sy: 1.25 });
    });

    it("is 1 when a size is not known yet", () => {
        expect(pointerScale({ width: 0, height: 0 }, OWN)).toEqual({ sx: 1, sy: 1 });
        expect(pointerScale(ZOOMED, { w: 0, h: 0 })).toEqual({ sx: 1, sy: 1 });
    });
});

describe("the lasso in a zoomed pane", () => {
    it("draws its box under the pointer, in the pane's own pixels", () => {
        createRoot((dispose) => {
            const lasso = useAtlasLasso({
                selecting: () => true,
                view: () => ({ x: 0, y: 0, k: 1 }),
                nodes: () => new Map(),
                onPick: () => {},
            });
            // 225 px right of and 125 px below the pane's corner on screen is 180 and 100 in the pane's own pixels.
            lasso.start(pointer(325, 175, "pointerdown"), viewportAt(ZOOMED));
            lasso.onPointerMove(pointer(725, 425));
            expect(lasso.rect()).toEqual({ x: 180, y: 100, w: 320, h: 200 });
            dispose();
        });
    });
});

describe("the wheel over the map", () => {
    function wheel(init: { ctrlKey?: boolean; metaKey?: boolean }) {
        let prevented = false;
        const e = {
            deltaY: -100,
            deltaMode: 0,
            clientX: 300,
            clientY: 200,
            ctrlKey: false,
            metaKey: false,
            ...init,
            preventDefault: () => (prevented = true),
        } as unknown as WheelEvent;
        return { e, prevented: () => prevented };
    }

    it("zooms the map, and with Ctrl or Cmd leaves the wheel to the pane zoom", () => {
        createRoot((dispose) => {
            const zoom = useAtlasPanZoom(
                () => ({ width: 800, height: 600 }),
                () => "machine",
                () => {}
            );
            zoom.setViewport(viewportAt(ZOOMED) as HTMLDivElement);
            const before = zoom.view();
            for (const key of [{ ctrlKey: true }, { metaKey: true }]) {
                const { e, prevented } = wheel(key);
                zoom.onWheel(e);
                expect(prevented()).toBe(false);
                expect(zoom.view()).toEqual(before);
            }
            const plain = wheel({});
            zoom.onWheel(plain.e);
            expect(plain.prevented()).toBe(true);
            expect(zoom.view().k).toBeGreaterThan(before.k);
            dispose();
        });
    });
});

describe("a zoom past the fitted view closes the rows and keeps its place", () => {
    const PLAIN = { left: 0, top: 0, width: 800, height: 600 };
    /** Three rows of one node each, as far apart as the layout's gap between a lead and its reports at `spread`. */
    const mapAt = (spread: number) => {
        const gap = gapsAt(spread).level;
        const nodes = new Map([0, 1, 2].map((row) => [`n${row}`, { x: 100, y: row * (36 + gap) }] as const));
        return { width: 400, height: 3 * 36 + 2 * gap, nodes };
    };
    function zoomed(run: (zoom: AtlasPanZoom, map: () => ReturnType<typeof mapAt>) => void) {
        let map!: () => ReturnType<typeof mapAt>;
        let zoom!: AtlasPanZoom;
        const dispose = createRoot((done) => {
            // As the view makes it: the map is laid out for the spread the zoom gives.
            zoom = useAtlasPanZoom(() => map(), () => "machine", () => {});
            map = createMemo(() => mapAt(zoom.spread()));
            zoom.setViewport({ ...viewportAt(PLAIN), clientWidth: 800, clientHeight: 600 } as unknown as HTMLDivElement);
            return done;
        });
        // The pane is measured and the map fitted once the root is made.
        run(zoom, map);
        dispose();
    }
    const wheelAt = (clientX: number, clientY: number, deltaY: number) =>
        ({ deltaY, deltaMode: 0, clientX, clientY, ctrlKey: false, metaKey: false, preventDefault: () => {} }) as unknown as WheelEvent;

    it("has the full gaps until the person zooms in, and closes them as they do", () => {
        zoomed((zoom, map) => {
            expect(zoom.spread()).toBe(1);
            const full = map().height;
            zoom.onWheel(wheelAt(300, 200, -400));
            expect(zoom.view().k).toBeGreaterThan(1);
            expect(zoom.spread()).toBeLessThan(1);
            expect(map().height).toBeLessThan(full);
            // Zoomed back out past where it started, the gaps are whole again.
            zoom.onWheel(wheelAt(300, 200, 2000));
            expect(zoom.spread()).toBe(1);
            expect(map().height).toBe(full);
        });
    });

    it("keeps the node under the pointer where it was, though the rows above it closed", () => {
        zoomed((zoom, map) => {
            const before = zoom.view();
            const node = map().nodes.get("n2")!;
            // The pointer is on the third row's node.
            const px = before.x + (node.x + 70) * before.k;
            const py = before.y + (node.y + 18) * before.k;
            zoom.onWheel(wheelAt(px, py, -400));
            const after = zoom.view();
            const moved = map().nodes.get("n2")!;
            expect(moved.y).toBeLessThan(node.y);
            expect(after.x + (moved.x + 70) * after.k).toBeCloseTo(px);
            expect(after.y + (moved.y + 18) * after.k).toBeCloseTo(py);
        });
    });

    it("centres the map at the size it has after a zoom button's step", () => {
        zoomed((zoom, map) => {
            zoom.zoomBy(1.25);
            zoom.zoomBy(1.25);
            const v = zoom.view();
            expect(zoom.spread()).toBeLessThan(1);
            expect(v.x + (map().width * v.k) / 2).toBeCloseTo(400);
            expect(v.y + (map().height * v.k) / 2).toBeCloseTo(300);
        });
    });
});
