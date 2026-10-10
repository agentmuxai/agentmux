// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    actualSizeScale,
    clampPan,
    FIT,
    maxScale,
    panBy,
    showsPixels,
    toggleZoom,
    wheelFactor,
    zoomAt,
    zoomPercent,
    type ZoomBox,
} from "./media-zoom";

// A 2000×1000 render fitted into an 800×600 view: 800×400 on screen.
const box: ZoomBox = { fitWidth: 800, fitHeight: 400, viewWidth: 800, viewHeight: 600, naturalWidth: 2000, naturalHeight: 1000 };

describe("media zoom", () => {
    it("starts at fit, and fit is as far out as it goes", () => {
        expect(zoomAt(FIT, 0.5, { x: 0, y: 0 }, box)).toEqual(FIT);
        expect(zoomPercent(FIT, box)).toBe(40);
    });

    it("keeps the point under the cursor where it is", () => {
        const cursor = { x: 200, y: -100 };
        const z = zoomAt(FIT, 2, cursor, box);
        expect(z.scale).toBe(2);
        // The image point under the cursor before (cursor - pan) / scale is
        // still under it after.
        expect((cursor.x - z.x) / z.scale).toBeCloseTo(cursor.x / 1);
        expect((cursor.y - z.y) / z.scale).toBeCloseTo(cursor.y / 1);
    });

    it("stops at 32 image pixels per screen pixel", () => {
        expect(maxScale(box)).toBe(80);
        expect(zoomAt(FIT, 1000, { x: 0, y: 0 }, box).scale).toBe(80);
        // A tiny image still gets at least 4× its fitted size.
        expect(maxScale({ ...box, naturalWidth: 50, naturalHeight: 25, fitWidth: 50, fitHeight: 25 })).toBe(32);
        expect(maxScale({ ...box, naturalWidth: 400, fitWidth: 400 * 10 })).toBe(4);
    });

    it("zooms by about 20% a wheel notch, in, out, and in lines or pages", () => {
        expect(wheelFactor(-100, 0)).toBeCloseTo(1.197, 2);
        expect(wheelFactor(100, 0)).toBeCloseTo(1 / 1.197, 2);
        expect(wheelFactor(-3, 1)).toBeCloseTo(wheelFactor(-120, 0));
        expect(wheelFactor(-1, 2)).toBe(wheelFactor(-300, 0));
        // A trackpad's few pixels zoom a little.
        expect(wheelFactor(-4, 0)).toBeGreaterThan(1);
        expect(wheelFactor(-4, 0)).toBeLessThan(1.01);
    });

    it("can't pan the image out of view", () => {
        const zoomed = { scale: 2, x: 0, y: 0 };
        // 1600×800 in an 800×600 view: 400 px of slack sideways, 100 down.
        expect(panBy(zoomed, 5000, -5000, box)).toEqual({ scale: 2, x: 400, y: -100 });
        expect(panBy(FIT, 50, 50, box)).toEqual(FIT);
        expect(clampPan({ scale: 1.2, x: 30, y: 30 }, box)).toEqual({ scale: 1.2, x: 30, y: 0 });
    });

    it("double-click goes to actual size at the cursor, and back to fit", () => {
        expect(actualSizeScale(box)).toBe(2.5);
        const z = toggleZoom(FIT, { x: 0, y: 0 }, box);
        expect(z.scale).toBe(2.5);
        expect(zoomPercent(z, box)).toBe(100);
        expect(toggleZoom(z, { x: 0, y: 0 }, box)).toEqual(FIT);
        // An image already at its own size doubles instead.
        const small = { ...box, naturalWidth: 800, naturalHeight: 400 };
        expect(toggleZoom(FIT, { x: 0, y: 0 }, small).scale).toBe(2);
    });

    it("shows pixels as squares once each is over 2 screen pixels", () => {
        expect(showsPixels({ scale: 5, x: 0, y: 0 }, box)).toBe(false);
        expect(showsPixels({ scale: 6, x: 0, y: 0 }, box)).toBe(true);
    });
});
