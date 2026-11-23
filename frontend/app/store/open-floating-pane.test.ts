// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";
import { openFloatingPane, workAreaOf, type FloaterWindow } from "./open-floating-pane";
import type { OpenFloatingPaneEvent } from "@/types/rpc/OpenFloatingPaneEvent";

const win = (over: Partial<FloaterWindow> = {}): FloaterWindow => ({
    screenX: 100,
    screenY: 80,
    outerWidth: 1400,
    outerHeight: 900,
    innerWidth: 1400,
    innerHeight: 860,
    devicePixelRatio: 1.5,
    screen: { availLeft: 0, availTop: 40, availWidth: 1920, availHeight: 1040 },
    isWindows: true,
    ...over,
});

const deps = () => ({ openFloatingPane: vi.fn().mockResolvedValue({ window_label: "floating-1" }), answer: vi.fn().mockResolvedValue(undefined) });

const event = (place?: OpenFloatingPaneEvent["place"]): OpenFloatingPaneEvent => ({ block_id: "b1", workspace_id: "w1", place });

describe("the person's own floating open (no placement)", () => {
    it("opens where it always did: about half the window, centered over it, and is not answered", async () => {
        const d = deps();
        await openFloatingPane(event(), win(), d);
        // 700 x 516, centered over the 1400 x 900 window at (100, 80); Windows takes physical pixels.
        expect(d.openFloatingPane).toHaveBeenCalledWith({
            pane_id: "b1",
            workspace_id: "w1",
            x: Math.round((100 + 350) * 1.5),
            y: Math.round((80 + (900 - 516) / 2) * 1.5),
            width: 700,
            height: 516,
        });
        expect(d.answer).not.toHaveBeenCalled();
    });

    it("takes DIP coordinates on macOS and Linux", async () => {
        const d = deps();
        await openFloatingPane(event(), win({ isWindows: false }), d);
        expect(d.openFloatingPane.mock.calls[0][0]).toMatchObject({ x: 450, y: 272 });
    });
});

describe("an agent's floating open", () => {
    const place = { request_id: "req-1", width: 700, height: 480, spot: "right", cascade: 0, activate: false };

    it("is shown without taking the foreground, at the place the spot names in this monitor's work area", async () => {
        const d = deps();
        await openFloatingPane(event(place), win({ isWindows: false }), d);
        expect(d.openFloatingPane).toHaveBeenCalledWith({
            pane_id: "b1",
            workspace_id: "w1",
            x: 1220,
            y: 320,
            width: 700,
            height: 480,
            activate: false,
        });
    });

    it("hands the host physical pixels on Windows and answers srv in CSS pixels from the work area", async () => {
        const d = deps();
        await openFloatingPane(event(place), win(), d);
        expect(d.openFloatingPane.mock.calls[0][0]).toMatchObject({ x: 1220 * 1.5, y: 320 * 1.5, width: 700, height: 480 });
        expect(d.answer).toHaveBeenCalledWith({
            request_id: "req-1",
            placed: { x: 1220, y: 280, width: 700, height: 480, spot: "right", clamped: false, display_scale: 1.5 },
        });
    });

    it("answers that it was pulled in when the request did not fit", async () => {
        const d = deps();
        await openFloatingPane(event({ ...place, spot: undefined, x: 9000, y: 0 }), win(), d);
        expect(d.answer.mock.calls[0][0].placed).toMatchObject({ x: 1220, y: 0, clamped: true });
    });

    it("answers with the reason when the host could not open the window, and says so", async () => {
        const d = deps();
        d.openFloatingPane.mockRejectedValue(new Error("a pane is closing"));
        await expect(openFloatingPane(event(place), win(), d)).rejects.toThrow("a pane is closing");
        expect(d.answer).toHaveBeenCalledWith({ request_id: "req-1", error: expect.stringContaining("a pane is closing") });
    });
});

describe("the work area", () => {
    it("is the one of the monitor the window is on, with an origin that may not be the screen's", () => {
        expect(workAreaOf(win({ screen: { availLeft: -1920, availTop: 0, availWidth: 1920, availHeight: 1000 } }))).toEqual({ x: -1920, y: 0, width: 1920, height: 1000 });
        expect(workAreaOf(win({ screen: { availWidth: 1280, availHeight: 720 } }))).toEqual({ x: 0, y: 0, width: 1280, height: 720 });
    });
});
