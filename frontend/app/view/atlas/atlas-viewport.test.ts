// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { fitView, localPoint, MAX_ZOOM, MIN_ZOOM, wheelFactor, zoomAt, zoomCentered } from "./atlas-viewport";

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

    it("stays centred when the map is larger than the pane, and stops at the limits", () => {
        const v = zoomCentered({ x: 0, y: 0, k: 1 }, 2, 800, 600, 400, 300);
        expect(v.x + (800 * v.k) / 2).toBeCloseTo(200);
        expect(v.y + (600 * v.k) / 2).toBeCloseTo(150);
        expect(zoomCentered({ x: 0, y: 0, k: 2 }, 100, 10, 10, 100, 100).k).toBe(MAX_ZOOM);
    });
});

describe("wheelFactor", () => {
    it("zooms in for an upward wheel and out for a downward one, by the same amount", () => {
        const up = wheelFactor({ deltaY: -100, deltaMode: 0, ctrlKey: false });
        const down = wheelFactor({ deltaY: 100, deltaMode: 0, ctrlKey: false });
        expect(up).toBeGreaterThan(1);
        expect(up * down).toBeCloseTo(1);
    });

    it("counts a wheel that steps in lines like one that steps in pixels", () => {
        const lines = wheelFactor({ deltaY: -3, deltaMode: 1, ctrlKey: false });
        // Three lines are 48 pixels, not 3.
        expect(lines).toBeCloseTo(wheelFactor({ deltaY: -48, deltaMode: 0, ctrlKey: false }));
    });

    it("makes a pinch (the wheel with ctrl, small deltas) zoom visibly", () => {
        expect(wheelFactor({ deltaY: -5, deltaMode: 0, ctrlKey: true })).toBeGreaterThan(1.04);
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
