// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, describe, expect, it } from "vitest";
import {
    __resetPaneRectRegistry,
    anyPaneIntersects,
    paneCount,
    registerPaneRect,
    unregisterPaneRect,
} from "./pane-rect-registry";

describe("pane-rect-registry", () => {
    afterEach(() => __resetPaneRectRegistry());

    it("paneCount reflects register / unregister", () => {
        expect(paneCount()).toBe(0);
        registerPaneRect("a", { x: 0, y: 0, w: 100, h: 100 });
        expect(paneCount()).toBe(1);
        registerPaneRect("b", { x: 200, y: 0, w: 100, h: 100 });
        expect(paneCount()).toBe(2);
        unregisterPaneRect("a");
        expect(paneCount()).toBe(1);
        unregisterPaneRect("a"); // idempotent
        expect(paneCount()).toBe(1);
    });

    it("anyPaneIntersects: true when overlay overlaps a registered pane", () => {
        registerPaneRect("a", { x: 100, y: 100, w: 200, h: 200 });
        // overlay rect fully inside the pane
        expect(anyPaneIntersects({ x: 150, y: 150, w: 50, h: 50 })).toBe(true);
        // overlay rect crossing the pane's left edge
        expect(anyPaneIntersects({ x: 80, y: 150, w: 50, h: 50 })).toBe(true);
        // overlay rect crossing the pane's top-right corner
        expect(anyPaneIntersects({ x: 290, y: 90, w: 50, h: 30 })).toBe(true);
    });

    it("anyPaneIntersects: false when overlay is fully outside every pane", () => {
        registerPaneRect("a", { x: 100, y: 100, w: 200, h: 200 });
        // overlay above the pane
        expect(anyPaneIntersects({ x: 100, y: 0, w: 200, h: 50 })).toBe(false);
        // overlay below the pane
        expect(anyPaneIntersects({ x: 100, y: 400, w: 200, h: 50 })).toBe(false);
        // overlay to the left
        expect(anyPaneIntersects({ x: 0, y: 100, w: 50, h: 200 })).toBe(false);
        // overlay to the right
        expect(anyPaneIntersects({ x: 400, y: 100, w: 50, h: 200 })).toBe(false);
    });

    it("anyPaneIntersects: edge-touching rects do NOT intersect (boundary inclusive on one side)", () => {
        registerPaneRect("a", { x: 100, y: 100, w: 200, h: 200 });
        // overlay's right edge touches the pane's left edge — no overlap
        expect(anyPaneIntersects({ x: 50, y: 150, w: 50, h: 50 })).toBe(false);
        // overlay's bottom edge touches the pane's top edge — no overlap
        expect(anyPaneIntersects({ x: 150, y: 50, w: 50, h: 50 })).toBe(false);
    });

    it("anyPaneIntersects: returns true if ANY of the registered panes intersects", () => {
        registerPaneRect("a", { x: 0, y: 0, w: 50, h: 50 });
        registerPaneRect("b", { x: 1000, y: 1000, w: 100, h: 100 });
        // overlay near pane b only
        expect(anyPaneIntersects({ x: 1050, y: 1050, w: 30, h: 30 })).toBe(true);
        // overlay matching pane a
        expect(anyPaneIntersects({ x: 25, y: 25, w: 10, h: 10 })).toBe(true);
    });

    it("anyPaneIntersects: zero-sized overlay rect → false", () => {
        registerPaneRect("a", { x: 0, y: 0, w: 100, h: 100 });
        expect(anyPaneIntersects({ x: 50, y: 50, w: 0, h: 0 })).toBe(false);
        expect(anyPaneIntersects({ x: 50, y: 50, w: 10, h: 0 })).toBe(false);
        expect(anyPaneIntersects({ x: 50, y: 50, w: 0, h: 10 })).toBe(false);
    });

    it("registerPaneRect overwrites prior rect for the same blockId", () => {
        registerPaneRect("a", { x: 0, y: 0, w: 50, h: 50 });
        expect(anyPaneIntersects({ x: 25, y: 25, w: 10, h: 10 })).toBe(true);
        // pane moves entirely off-screen
        registerPaneRect("a", { x: 9999, y: 9999, w: 50, h: 50 });
        expect(anyPaneIntersects({ x: 25, y: 25, w: 10, h: 10 })).toBe(false);
        // still one pane registered (overwrite, not append)
        expect(paneCount()).toBe(1);
    });

    it("anyPaneIntersects: empty registry → false", () => {
        expect(anyPaneIntersects({ x: 0, y: 0, w: 100, h: 100 })).toBe(false);
    });
});
