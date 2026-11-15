// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot } from "solid-js";
import { describe, expect, it } from "vitest";
import { useAtlasLasso } from "./atlas-lasso";
import { useAtlasPanZoom } from "./atlas-pan-zoom";
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
