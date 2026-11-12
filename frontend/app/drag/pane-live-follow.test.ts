// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";

const hub = vi.hoisted(() => ({
    shot: undefined as unknown,
    begun: [] as unknown[],
    resized: [] as unknown[],
    captured: [] as unknown[],
    /** What the host answers a capture with, and what runs while it is being taken. */
    staged: true,
    whileCapturing: () => {},
}));

vi.mock("@/store/global", () => ({ getApi: () => ({ marker: "api" }) }));
vi.mock("./tearoff-snapshot", () => ({ peekTearOffSnapshot: async () => hub.shot }));
vi.mock("./live-follow", () => ({
    beginLiveFollow: async (_api: unknown, spec: unknown) => void hub.begun.push(spec),
    resizeLiveFollow: async (_api: unknown, size: object, landing?: object) => void hub.resized.push({ ...size, ...landing }),
    captureLiveFollow: async (_api: unknown, capture: unknown, wanted: () => boolean) => {
        if (!wanted()) return false;
        hub.captured.push(capture);
        hub.whileCapturing();
        return hub.staged;
    },
}));

import { followPaneDrag, followSize, HAND_SCALE, landingKey } from "./pane-live-follow";

beforeEach(() => {
    hub.begun = [];
    hub.resized = [];
    hub.captured = [];
    hub.staged = true;
    hub.whileCapturing = () => {};
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
        expect(hub.resized).toEqual([{ width: 140, height: 210, landingKey: "200x300" }]);
        // The same rect again sends nothing.
        setRect({ width: 200, height: 300 });
        expect(hub.resized).toHaveLength(1);
        // A smaller, square one.
        setRect({ width: 100, height: 100 });
        expect(hub.resized[1]).toEqual({ width: 70, height: 70, landingKey: "100x100" });
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

    it("sends a larger landing rect of the same shape as another one: the pane is laid out differently in it", async () => {
        // Both are larger than the pane, so the hand's size is the same; the pictures are not.
        const [rect, setRect] = createSignal<{ width: number; height: number } | undefined>({ width: 600, height: 400 });
        await followPaneDrag("b1", "main", { rect, active: () => true });
        setRect({ width: 900, height: 600 });
        expect(hub.resized).toEqual([
            { width: 210, height: 140, landingKey: "600x400" },
            { width: 210, height: 140, landingKey: "900x600" },
        ]);
        expect(landingKey({ width: 599.6, height: 400.4 })).toBe("600x400");
    });
});

describe("the pane's picture in its landing rect", () => {
    const site = { rect: { left: 10, top: 20, width: 194, height: 294 }, viewportWidth: 1280, wash: { r: 65, g: 159, b: 224, a: 0.26 } };

    /** A drag over a 200 by 300 landing rect whose panes come to rest when `rest` is called. */
    async function dragOver() {
        const [rect, setRect] = createSignal<{ width: number; height: number } | undefined>({ width: 200, height: 300 });
        const [settled, setSettled] = createSignal<object | undefined>(undefined);
        const [active, setActive] = createSignal(true);
        await followPaneDrag("b1", "main", { rect, active, settled, site: () => site });
        return { setRect, setActive, rest: () => setSettled({}), move: () => setSettled(undefined) };
    }
    const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

    it("is taken once the panes rest on the drop, at the hand's size, and shown in place of the resized one", async () => {
        const drag = await dragOver();
        expect(hub.captured).toEqual([]);
        drag.rest();
        await flush();
        expect(hub.captured).toEqual([{ landingKey: "200x300", size: { width: 140, height: 210 }, ...site }]);
        expect(hub.resized.at(-1)).toEqual({ width: 140, height: 210, landingKey: "200x300", adopt: true });
    });

    it("is not shown when the host took none: it has that rect's picture already, or the capture failed", async () => {
        hub.staged = false;
        const drag = await dragOver();
        drag.rest();
        await flush();
        expect(hub.captured).toHaveLength(1);
        expect(hub.resized).toEqual([{ width: 140, height: 210, landingKey: "200x300" }]);
    });

    it("is thrown away when the panes moved on while it was being taken: it is a picture of panes in motion", async () => {
        const drag = await dragOver();
        hub.whileCapturing = () => drag.move();
        drag.rest();
        await flush();
        expect(hub.captured).toHaveLength(1);
        expect(hub.resized.some((r) => (r as { adopt?: boolean }).adopt)).toBe(false);
    });

    it("is not taken for a drop the drag ended on before the panes had rested", async () => {
        const drag = await dragOver();
        drag.setActive(false);
        drag.rest();
        await flush();
        expect(hub.captured).toEqual([]);
    });

    it("is not taken where the pane cannot be read on screen", async () => {
        const [settled, setSettled] = createSignal<object | undefined>(undefined);
        await followPaneDrag("b1", "main", { rect: () => ({ width: 200, height: 300 }), active: () => true, settled, site: () => undefined });
        setSettled({});
        await flush();
        expect(hub.captured).toEqual([]);
    });
});
