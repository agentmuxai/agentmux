// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";

const hub = vi.hoisted(() => ({
    shot: undefined as unknown,
    begun: [] as unknown[],
    resized: [] as unknown[],
}));

vi.mock("@/store/global", () => ({ getApi: () => ({ marker: "api" }) }));
vi.mock("./tearoff-snapshot", () => ({ peekTearOffSnapshot: async () => hub.shot }));
vi.mock("./live-follow", () => ({
    beginLiveFollow: async (_api: unknown, spec: unknown) => void hub.begun.push(spec),
    resizeLiveFollow: async (_api: unknown, size: unknown) => void hub.resized.push(size),
}));

import { followPaneDrag, followSize, MIN_FOLLOW_SCALE } from "./pane-live-follow";

beforeEach(() => {
    hub.begun = [];
    hub.resized = [];
    hub.shot = { picture: "jpeg", grab: { x: 50, y: 15 }, size: { width: 300, height: 200 } };
});

describe("followPaneDrag", () => {
    it("gives the host the pane's picture at its size and the pointer's grab, with no tear line", async () => {
        await followPaneDrag("b1", "main");
        expect(hub.begun).toEqual([
            {
                sourceWindowLabel: "main",
                picture: "jpeg",
                windowWidth: 300,
                windowHeight: 200,
                offsetX: 50,
                offsetY: 15,
            },
        ]);
        expect(hub.resized).toEqual([]);
    });

    it("shows nothing when the pane has no picture in time", async () => {
        hub.shot = undefined;
        await followPaneDrag("b1", "main");
        expect(hub.begun).toEqual([]);
    });

    it("resizes the whole picture to fit a smaller landing rect, never pads it to a larger one, and lets go when the drag ends", async () => {
        // The pane is 300 by 200. A landing rect wider and shorter: the pane is made smaller in its own
        // proportions until its height fits.
        const [rect, setRect] = createSignal<{ width: number; height: number } | undefined>({ width: 1200.4, height: 120 });
        const [active, setActive] = createSignal(true);
        // A drop is already armed when the picture is up: it is resized at once.
        await followPaneDrag("b1", "main", { rect, active });
        expect(hub.resized).toEqual([{ width: 180, height: 120 }]);
        // The same size again sends nothing.
        setRect({ width: 900, height: 120 });
        expect(hub.resized).toHaveLength(1);
        // Smaller on both axes: the tighter one decides.
        setRect({ width: 150, height: 150 });
        expect(hub.resized[1]).toEqual({ width: 150, height: 100 });
        // Larger on both: the pane's own size, with no blank band beside it.
        setRect({ width: 2000, height: 1000 });
        expect(hub.resized[2]).toEqual({ width: 300, height: 200 });
        // No drop armed: still the pane's own size, so nothing is sent.
        setRect(undefined);
        expect(hub.resized).toHaveLength(3);
        // The drag ended: a later rect is ignored.
        setActive(false);
        setRect({ width: 100, height: 100 });
        expect(hub.resized).toHaveLength(3);
    });

    it("keeps the pane's proportions, never grows it, and never makes it too small to be a pane", () => {
        const own = { width: 600, height: 400 };
        expect(followSize(own)).toEqual(own);
        expect(followSize(own, { width: 2000, height: 1000 })).toEqual(own);
        expect(followSize(own, { width: 300, height: 400 })).toEqual({ width: 300, height: 200 });
        expect(followSize(own, { width: 600, height: 300 })).toEqual({ width: 450, height: 300 });
        // A sliver of a landing rect: the floor.
        expect(followSize(own, { width: 30, height: 400 })).toEqual({ width: 600 * MIN_FOLLOW_SCALE, height: 400 * MIN_FOLLOW_SCALE });
    });
});
