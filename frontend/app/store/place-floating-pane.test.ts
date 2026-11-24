// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import type { FloaterWindow } from "./open-floating-pane";
import { placeFloatingPane } from "./place-floating-pane";
import type { PlaceFloatingPaneEvent } from "@/types/rpc/PlaceFloatingPaneEvent";

// A 1920 x 1040 work area at (0, 40), 150% scale on Windows (so the host speaks physical pixels).
const win = (over: Partial<FloaterWindow> = {}): FloaterWindow => ({
    screenX: 0,
    screenY: 0,
    outerWidth: 800,
    outerHeight: 600,
    innerWidth: 800,
    innerHeight: 600,
    devicePixelRatio: 1,
    screen: { availLeft: 0, availTop: 40, availWidth: 1920, availHeight: 1040 },
    isWindows: false,
    ...over,
});

const deps = (rect = { x: 300, y: 200, width: 900, height: 600 }) => ({
    getRect: vi.fn().mockResolvedValue(rect),
    setRect: vi.fn().mockResolvedValue(undefined),
    answer: vi.fn().mockResolvedValue(undefined),
});

const place = (over: Record<string, unknown> = {}) => ({ request_id: "req-1", cascade: 0, activate: false, ...over });
const event = (over: Partial<PlaceFloatingPaneEvent> = {}): PlaceFloatingPaneEvent => ({ request_id: "req-1", block_id: "b1", ...over });

describe("where a floating window is, with nothing to change", () => {
    it("answers its rect from the work area's top-left and sets nothing", async () => {
        const d = deps();
        await placeFloatingPane(event(), win(), d);
        expect(d.setRect).not.toHaveBeenCalled();
        expect(d.answer).toHaveBeenCalledWith({
            request_id: "req-1",
            placed: { x: 300, y: 160, width: 900, height: 600, spot: undefined, clamped: false, display_scale: 1 },
        });
    });

    it("converts the host's physical pixels to CSS pixels on Windows", async () => {
        const d = deps({ x: 450, y: 360, width: 1350, height: 900 });
        await placeFloatingPane(event(), win({ isWindows: true, devicePixelRatio: 1.5 }), d);
        expect(d.answer.mock.calls[0][0].placed).toMatchObject({ x: 300, y: 200, width: 900, height: 600, display_scale: 1.5 });
    });

    it("says why when the host has no window to read, instead of an invented rect", async () => {
        const d = deps({ x: 0, y: 0, width: 0, height: 0 });
        await placeFloatingPane(event({ place: place({ width: 700 }) }), win(), d);
        expect(d.setRect).not.toHaveBeenCalled();
        expect(d.answer).toHaveBeenCalledWith({ request_id: "req-1", error: "the window's size could not be read" });
    });
});

describe("a resize", () => {
    it("changes the size and keeps the position", async () => {
        const d = deps();
        await placeFloatingPane(event({ keep: "position", place: place({ width: 700, height: "50%" }) }), win(), d);
        expect(d.setRect).toHaveBeenCalledWith({ x: 300, y: 200, width: 700, height: 520 });
        expect(d.answer.mock.calls[0][0].placed).toMatchObject({ x: 300, y: 160, width: 700, height: 520, clamped: false });
    });

    it("keeps the other side of the size when only one is given", async () => {
        const d = deps();
        await placeFloatingPane(event({ keep: "position", place: place({ width: 640 }) }), win(), d);
        expect(d.setRect).toHaveBeenCalledWith({ x: 300, y: 200, width: 640, height: 600 });
    });

    it("pulls a window that would run off the work area back in, and says it was clamped", async () => {
        const d = deps({ x: 1500, y: 200, width: 400, height: 400 });
        await placeFloatingPane(event({ keep: "position", place: place({ width: 800 }) }), win(), d);
        expect(d.setRect).toHaveBeenCalledWith({ x: 1120, y: 200, width: 800, height: 400 });
        expect(d.answer.mock.calls[0][0].placed.clamped).toBe(true);
    });

    it("raises a size below the minimum to it", async () => {
        const d = deps();
        await placeFloatingPane(event({ keep: "position", place: place({ width: 100, height: 100 }) }), win(), d);
        expect(d.setRect).toHaveBeenCalledWith({ x: 300, y: 200, width: 320, height: 200 });
        expect(d.answer.mock.calls[0][0].placed.clamped).toBe(true);
    });
});

describe("a move", () => {
    it("goes to a named spot and keeps the size", async () => {
        const d = deps();
        await placeFloatingPane(event({ keep: "size", place: place({ spot: "top-right" }) }), win(), d);
        expect(d.setRect).toHaveBeenCalledWith({ x: 1020, y: 40, width: 900, height: 600 });
        expect(d.answer.mock.calls[0][0].placed).toMatchObject({ x: 1020, y: 0, width: 900, height: 600, spot: "top-right" });
    });

    it("goes to an exact x and y from the work area's top-left", async () => {
        const d = deps();
        await placeFloatingPane(event({ keep: "size", place: place({ x: 50, y: 60 }) }), win(), d);
        expect(d.setRect).toHaveBeenCalledWith({ x: 50, y: 100, width: 900, height: 600 });
    });

    it("speaks physical pixels to the host on Windows and CSS pixels to the agent", async () => {
        const d = deps({ x: 450, y: 360, width: 1350, height: 900 });
        await placeFloatingPane(event({ keep: "size", place: place({ x: 100, y: 100 }) }), win({ isWindows: true, devicePixelRatio: 1.5 }), d);
        expect(d.setRect).toHaveBeenCalledWith({ x: 150, y: 210, width: 1350, height: 900 });
        expect(d.answer.mock.calls[0][0].placed).toMatchObject({ x: 100, y: 100, width: 900, height: 600, display_scale: 1.5 });
    });

    it("pulls a position outside the work area in and says so", async () => {
        const d = deps();
        await placeFloatingPane(event({ keep: "size", place: place({ x: 5000, y: 5000 }) }), win(), d);
        expect(d.setRect).toHaveBeenCalledWith({ x: 1020, y: 480, width: 900, height: 600 });
        expect(d.answer.mock.calls[0][0].placed.clamped).toBe(true);
    });
});

describe("a place", () => {
    it("sets size and position in one call", async () => {
        const d = deps();
        await placeFloatingPane(event({ place: place({ width: 1000, height: 700, spot: "left" }) }), win(), d);
        expect(d.setRect).toHaveBeenCalledTimes(1);
        expect(d.setRect).toHaveBeenCalledWith({ x: 0, y: 210, width: 1000, height: 700 });
    });

    it("never cascades or caps a size an agent gave", async () => {
        const d = deps();
        await placeFloatingPane(event({ place: place({ width: 1900, height: 1000, x: 0, y: 0, cascade: 3, max_share: 0.5 }) }), win(), d);
        expect(d.setRect).toHaveBeenCalledWith({ x: 0, y: 40, width: 1900, height: 1000 });
    });
});

describe("a host that refuses", () => {
    it("answers with the error and rethrows it", async () => {
        const d = deps();
        d.setRect.mockRejectedValue(new Error("no such window"));
        await expect(placeFloatingPane(event({ keep: "size", place: place({ spot: "left" }) }), win(), d)).rejects.toThrow("no such window");
        expect(d.answer).toHaveBeenCalledWith({ request_id: "req-1", error: "Error: no such window" });
    });
});
