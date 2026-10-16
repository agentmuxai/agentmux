// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    actualSizeScale,
    atMaxZoom,
    clampPan,
    clickSteps,
    FIT,
    maxScale,
    panBy,
    showsPixels,
    wheelFactor,
    zoomAt,
    zoomPercent,
    zoomStep,
    type ZoomBox,
} from "./image-zoom";

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

    it("goes to actual size at 100%", () => {
        expect(actualSizeScale(box)).toBe(2.5);
        expect(zoomPercent(zoomAt(FIT, 2.5, { x: 0, y: 0 }, box), box)).toBe(100);
    });

    it("clicks through five even steps from fit to the maximum", () => {
        const steps = clickSteps(box);
        expect(steps).toHaveLength(6);
        expect(steps[0]).toBe(1);
        expect(steps[5]).toBe(80);
        // Each step the same factor: 80^(1/5), about 2.4.
        for (let k = 1; k < 6; k++) expect(steps[k] / steps[k - 1]).toBeCloseTo(80 ** 0.2);
        // An image the size of the pane: 32×, 2× a step.
        const pane = { ...box, naturalWidth: 800, naturalHeight: 400 };
        expect(clickSteps(pane)[1]).toBeCloseTo(2);
    });

    it("five clicks reach the deepest zoom, and five back reach fit", () => {
        let z = FIT;
        for (let k = 1; k <= 5; k++) {
            z = zoomStep(z, 1, { x: 0, y: 0 }, box);
            expect(z.scale).toBeCloseTo(clickSteps(box)[k]);
        }
        expect(atMaxZoom(z, box)).toBe(true);
        expect(zoomStep(z, 1, { x: 0, y: 0 }, box)).toBe(z);
        for (let k = 4; k >= 0; k--) {
            z = zoomStep(z, -1, { x: 0, y: 0 }, box);
            expect(z.scale).toBeCloseTo(clickSteps(box)[k]);
        }
        expect(z).toEqual(FIT);
        expect(atMaxZoom(z, box)).toBe(false);
        expect(zoomStep(FIT, -1, { x: 0, y: 0 }, box)).toBe(FIT);
    });

    it("from between steps, goes to the next step up or down", () => {
        const [, s1, s2] = clickSteps(box);
        const between = { scale: (s1 + s2) / 2, x: 0, y: 0 };
        expect(zoomStep(between, 1, { x: 0, y: 0 }, box).scale).toBeCloseTo(s2);
        expect(zoomStep(between, -1, { x: 0, y: 0 }, box).scale).toBeCloseTo(s1);
        // A rounding error off a step counts as on it: the click moves a whole step.
        const onStep = { scale: s1 * (1 + 1e-9), x: 0, y: 0 };
        expect(zoomStep(onStep, 1, { x: 0, y: 0 }, box).scale).toBeCloseTo(s2);
        expect(zoomStep(onStep, -1, { x: 0, y: 0 }, box)).toEqual(FIT);
    });

    it("a click keeps the point under the cursor where it is", () => {
        const cursor = { x: 150, y: 60 };
        const before = { scale: 2, x: 40, y: -20 };
        const z = zoomStep(before, 1, cursor, box);
        expect(z.scale).toBeGreaterThan(before.scale);
        expect((cursor.x - z.x) / z.scale).toBeCloseTo((cursor.x - before.x) / before.scale);
        expect((cursor.y - z.y) / z.scale).toBeCloseTo((cursor.y - before.y) / before.scale);
    });

    it("shows pixels as squares once each is over 2 screen pixels", () => {
        expect(showsPixels({ scale: 5, x: 0, y: 0 }, box)).toBe(false);
        expect(showsPixels({ scale: 6, x: 0, y: 0 }, box)).toBe(true);
    });
});
