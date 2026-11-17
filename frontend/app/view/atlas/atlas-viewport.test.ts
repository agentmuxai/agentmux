// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import {
    fitView,
    keepInPlace,
    localPoint,
    MAX_ZOOM,
    MIN_ZOOM,
    nearestPlaced,
    gapSpread,
    wheelFactor,
    zoomAt,
    zoomCentered,
} from "./atlas-viewport";

describe("zoomAt", () => {
    it("keeps the point under the pointer where it is", () => {
        const v = { x: 40, y: 10, k: 1 };
        const z = zoomAt(v, 200, 120, 2);
        // The map point under (200, 120) before and after.
        expect((200 - v.x) / v.k).toBeCloseTo((200 - z.x) / z.k);
        expect((120 - v.y) / v.k).toBeCloseTo((120 - z.y) / z.k);
        expect(z.k).toBe(2);
    });

    it("stops at the limits", () => {
        expect(zoomAt({ x: 0, y: 0, k: 2 }, 0, 0, 100).k).toBe(MAX_ZOOM);
        expect(zoomAt({ x: 0, y: 0, k: 0.2 }, 0, 0, 0.001).k).toBe(MIN_ZOOM);
    });
});

describe("fitView", () => {
    it("shrinks a large map to the pane, from the top left", () => {
        const v = fitView(2000, 1000, 1000, 600);
        expect(v.k).toBeLessThan(1);
        expect(2000 * v.k + v.x).toBeLessThanOrEqual(1000);
        expect(1000 * v.k + v.y).toBeLessThanOrEqual(600);
        expect(v).toMatchObject({ x: 16, y: 16 });
    });

    it("does not enlarge a small map", () => {
        expect(fitView(200, 100, 1000, 600).k).toBe(1);
    });

    it("has a default for an empty map or pane", () => {
        expect(fitView(0, 0, 1000, 600).k).toBe(1);
        expect(fitView(100, 100, 0, 0).k).toBe(1);
    });
});

describe("zoomCentered", () => {
    it("puts the map in the middle of the pane at the new scale, from any position", () => {
        const v = zoomCentered({ x: -300, y: 90, k: 1 }, 1.25, 800, 600, 1000, 700);
        expect(v.k).toBe(1.25);
        expect(v.x + (800 * v.k) / 2).toBeCloseTo(500);
        expect(v.y + (600 * v.k) / 2).toBeCloseTo(350);
    });

    it("leaves a map that still fits where it is, so the first zoom in from the fit does not slide it sideways", () => {
        // A 400 by 300 map fitted at the top left of a 1000 by 700 pane, zoomed in once.
        const v = zoomCentered({ x: 16, y: 16, k: 1 }, 1.25, 400, 300, 1000, 700);
        expect(v).toEqual({ k: 1.25, x: 16, y: 16 });
        // ...and out again.
        expect(zoomCentered(v, 0.8, 400, 300, 1000, 700)).toEqual({ k: 1, x: 16, y: 16 });
    });

    it("keeps a map that fits inside the pane when it grows toward the far edge", () => {
        const v = zoomCentered({ x: 500, y: 16, k: 1 }, 1.5, 400, 300, 1000, 700);
        // 600 wide now: its right side stops at the pane's margin, not past it.
        expect(v.x + 400 * v.k).toBeLessThanOrEqual(1000 - 16);
        expect(v.y).toBe(16);
    });

    it("centres a map that grows to fill the pane, with no jump where it stops fitting", () => {
        const fits = zoomCentered({ x: 16, y: 16, k: 1 }, 2.4, 400, 300, 1000, 700);
        const fills = zoomCentered({ x: 16, y: 16, k: 1 }, 2.5, 400, 300, 1000, 700);
        expect(fits.x).toBeGreaterThanOrEqual(16);
        expect(fills.x + (400 * fills.k) / 2).toBeCloseTo(500);
        // The pane's margin is the most it can move when the map stops fitting.
        expect(Math.abs(fills.x - fits.x)).toBeLessThanOrEqual(16);
    });

    it("stays centred when the map is larger than the pane, and stops at the limits", () => {
        const v = zoomCentered({ x: 0, y: 0, k: 1 }, 2, 800, 600, 400, 300);
        expect(v.x + (800 * v.k) / 2).toBeCloseTo(200);
        expect(v.y + (600 * v.k) / 2).toBeCloseTo(150);
        expect(zoomCentered({ x: 0, y: 0, k: 2 }, 100, 10, 10, 100, 100).k).toBe(MAX_ZOOM);
    });
});

describe("wheelFactor", () => {
    it("zooms in for an upward wheel and out for a downward one, by the same amount", () => {
        const up = wheelFactor({ deltaY: -100, deltaMode: 0 });
        const down = wheelFactor({ deltaY: 100, deltaMode: 0 });
        expect(up).toBeGreaterThan(1);
        expect(up * down).toBeCloseTo(1);
    });

    it("counts a wheel that steps in lines like one that steps in pixels", () => {
        const lines = wheelFactor({ deltaY: -3, deltaMode: 1 });
        // Three lines are 48 pixels, not 3.
        expect(lines).toBeCloseTo(wheelFactor({ deltaY: -48, deltaMode: 0 }));
    });
});

describe("localPoint", () => {
    it("is the offset from the corner when the pane is not zoomed", () => {
        expect(localPoint(300, 200, { left: 100, top: 50, width: 640, height: 500 }, { w: 640, h: 500 })).toEqual({
            x: 200,
            y: 150,
        });
    });

    it("divides by the pane's zoom, so the point is in the map's own pixels", () => {
        // The pane draws 1.25 times larger than its own size: 250 screen pixels in is 200 of its own.
        const p = localPoint(350, 175, { left: 100, top: 50, width: 800, height: 625 }, { w: 640, h: 500 });
        expect(p.x).toBeCloseTo(200);
        expect(p.y).toBeCloseTo(100);
    });
});

describe("gapSpread", () => {
    it("is all of the gaps up to the fitted view, and past it the share that keeps a gap its size on screen", () => {
        expect(gapSpread(0.8, 0.8)).toBe(1);
        expect(gapSpread(0.8, 0.5)).toBe(1);
        expect(gapSpread(0.8, 1.6)).toBeCloseTo(0.5);
        // A gap of 40 map pixels is 32 on screen in the fitted view, and still 32 at twice the scale.
        expect(40 * gapSpread(0.8, 1.6) * 1.6).toBeCloseTo(40 * 0.8);
        expect(gapSpread(0, 2)).toBe(1);
    });
});

describe("nearestPlaced and keepInPlace", () => {
    const placed = new Map([
        ["a", { x: 0, y: 0 }],
        ["b", { x: 200, y: 0 }],
        ["c", { x: 0, y: 300 }],
    ]);

    it("finds what stands nearest a map point, and nothing on an empty map", () => {
        expect(nearestPlaced(placed, 190, 20)).toBe("b");
        expect(nearestPlaced(placed, 10, 260)).toBe("c");
        expect(nearestPlaced(new Map(), 0, 0)).toBeUndefined();
    });

    it("moves the view so that what moved in the map is where it was in the pane", () => {
        const view = { x: 40, y: 10, k: 2 };
        const was = { x: 0, y: 300 };
        const now = { x: 0, y: 180 };
        const kept = keepInPlace(view, was, now);
        expect(kept.y + now.y * kept.k).toBeCloseTo(view.y + was.y * view.k);
        expect(kept.x + now.x * kept.k).toBeCloseTo(view.x + was.x * view.k);
        expect(kept.k).toBe(2);
    });
});
