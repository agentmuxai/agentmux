// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { beforeEach, describe, expect, it, vi } from "vitest";
import { createSignal } from "solid-js";

const hub = vi.hoisted(() => ({
    shot: undefined as unknown,
    begun: [] as unknown[],
    quiets: [] as boolean[],
    fades: [] as boolean[],
}));

vi.mock("@/store/global", () => ({ getApi: () => ({ marker: "api" }) }));
vi.mock("./tearoff-snapshot", () => ({ peekTearOffSnapshot: async () => hub.shot }));
vi.mock("./live-follow", () => ({
    beginLiveFollow: async (_api: unknown, spec: unknown) => void hub.begun.push(spec),
    quietLiveFollow: async (_api: unknown, on: boolean) => void hub.quiets.push(on),
    fadeLiveFollow: async (_api: unknown, on: boolean) => void hub.fades.push(on),
}));

import { setPlatform } from "@/util/platformutil";
import { fadeFollowedPane, followPaneDrag } from "./pane-live-follow";

const BODY = { left: 0, top: 33, width: 1200, height: 744 };
const PICTURE = { sourceWindowLabel: "main", picture: "jpeg", windowWidth: 300, windowHeight: 200, offsetX: 50, offsetY: 15 };

beforeEach(() => {
    hub.begun = [];
    hub.quiets = [];
    hub.shot = { picture: "jpeg", grab: { x: 50, y: 15 }, size: { width: 300, height: 200 } };
    vi.stubGlobal("window", { innerWidth: 1200 });
});

describe("followPaneDrag", () => {
    it("gives the host the pane's picture at the pane's own size and the pointer's grab, with no tear line", async () => {
        await followPaneDrag("b1", "main");
        expect(hub.begun).toEqual([PICTURE]);
        expect(hub.quiets).toEqual([]);
    });

    it("shows nothing when the pane has no picture in time", async () => {
        hub.shot = undefined;
        await followPaneDrag("b1", "main");
        expect(hub.begun).toEqual([]);
    });

    it("names the body of the layout the drag began in as where the picture stays out of sight", async () => {
        await followPaneDrag("b1", "main", { body: () => BODY, holdsPane: () => true, active: () => true });
        expect(hub.begun).toEqual([{ ...PICTURE, quiet: { ...BODY, viewportWidth: 1200 } }]);
        // The host starts with the area on: nothing more to say.
        expect(hub.quiets).toEqual([]);
    });

    it("shows the picture everywhere when the layout's body cannot be read", async () => {
        const [holds, setHolds] = createSignal(true);
        await followPaneDrag("b1", "main", { body: () => undefined, holdsPane: holds, active: () => true });
        expect(hub.begun).toEqual([PICTURE]);
        setHolds(false);
        expect(hub.quiets).toEqual([]);
    });

    it("lets the picture show over the body while the layout does not show the pane, and hides it again when it does, until the drag ends", async () => {
        const [holds, setHolds] = createSignal(true);
        const [active, setActive] = createSignal(true);
        await followPaneDrag("b1", "main", { body: () => BODY, holdsPane: holds, active });
        // The drag went on to another window tab, or the layout shows the tear-off.
        setHolds(false);
        expect(hub.quiets).toEqual([false]);
        // Back: the layout shows the pane again.
        setHolds(true);
        expect(hub.quiets).toEqual([false, true]);
        // The drag ended: a later change is nobody's.
        setActive(false);
        setHolds(false);
        expect(hub.quiets).toEqual([false, true]);
    });

    it("turns the area off at once for a layout that already shows no pane when the picture is up", async () => {
        await followPaneDrag("b1", "main", { body: () => BODY, holdsPane: () => false, active: () => true });
        expect(hub.quiets).toEqual([false]);
    });
});

describe("fadeFollowedPane", () => {
    it("fades the picture while a layout shows drop guides, on Windows, where there is a picture", () => {
        hub.fades = [];
        setPlatform("win32");
        fadeFollowedPane(true);
        fadeFollowedPane(false);
        expect(hub.fades).toEqual([true, false]);
        setPlatform("linux");
        fadeFollowedPane(true);
        expect(hub.fades).toEqual([true, false]);
        setPlatform("darwin");
    });
});
