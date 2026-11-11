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

import { followPaneDrag } from "./pane-live-follow";

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

    it("cuts the picture to a smaller landing rect, never pads it to a larger one, and lets go when the drag ends", async () => {
        // The pane is 300 by 200. A landing rect wider and shorter: only the height is cut.
        const [rect, setRect] = createSignal<{ width: number; height: number } | undefined>({ width: 1200.4, height: 120.3 });
        const [active, setActive] = createSignal(true);
        // A drop is already armed when the picture is up: it is cut at once.
        await followPaneDrag("b1", "main", { rect, active });
        expect(hub.resized).toEqual([{ width: 300, height: 120 }]);
        // The same size again sends nothing.
        setRect({ width: 900, height: 120 });
        expect(hub.resized).toHaveLength(1);
        // Smaller on both axes.
        setRect({ width: 150, height: 90 });
        expect(hub.resized[1]).toEqual({ width: 150, height: 90 });
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
});
