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

import { followPaneDrag, followSize, HAND_SCALE } from "./pane-live-follow";

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

    it("gives the picture the landing rect's shape, smaller than it, and its own size again with no drop, until the drag ends", async () => {
        // The pane is 300 by 200. An upright landing rect of the same area: its shape, at the hand's share.
        const [rect, setRect] = createSignal<{ width: number; height: number } | undefined>({ width: 200, height: 300 });
        const [active, setActive] = createSignal(true);
        // A drop is already armed when the picture is up: it is resized at once.
        await followPaneDrag("b1", "main", { rect, active });
        expect(hub.resized).toEqual([{ width: 140, height: 210 }]);
        // The same rect again sends nothing.
        setRect({ width: 200, height: 300 });
        expect(hub.resized).toHaveLength(1);
        // A smaller, square one.
        setRect({ width: 100, height: 100 });
        expect(hub.resized[1]).toEqual({ width: 70, height: 70 });
        // No drop armed: the pane's own size and shape.
        setRect(undefined);
        expect(hub.resized[2]).toEqual({ width: 300, height: 200 });
        // The drag ended: a later rect is ignored.
        setActive(false);
        setRect({ width: 100, height: 100 });
        expect(hub.resized).toHaveLength(3);
    });

    it("takes the landing rect's shape at the hand's share of the smaller of the two, the rect and the pane", () => {
        const own = { width: 600, height: 400 };
        expect(followSize(own)).toEqual(own);
        // A landing rect smaller than the pane: its shape, smaller by the share on both axes.
        expect(followSize(own, { width: 300, height: 400 })).toEqual({ width: 210, height: 280 });
        expect(HAND_SCALE).toBe(0.7);
        // The pane's own rect: the pane, smaller by the share.
        expect(followSize(own, own)).toEqual({ width: 420, height: 280 });
        // A larger one of another shape: that shape, with the area of the pane's share, not blown up to the rect.
        const upright = followSize(own, { width: 800, height: 1200 });
        expect(upright).toEqual({ width: 280, height: 420 });
        expect(upright.width * upright.height).toBe(420 * 280);
        // A rect with no size is no landing rect.
        expect(followSize(own, { width: 0, height: 400 })).toEqual(own);
    });
});
