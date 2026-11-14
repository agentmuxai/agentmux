// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { fitView, MAX_ZOOM, MIN_ZOOM, zoomAt } from "./atlas-viewport";

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
