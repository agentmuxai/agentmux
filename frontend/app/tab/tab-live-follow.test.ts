// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    api: {
        startTabDragTracking: vi.fn(async () => {}),
    },
    picture: undefined as string | undefined,
    offset: { x: 130, y: 25 } as { x: number; y: number } | null,
    begun: [] as unknown[],
    caps: { tearOff: true } as Record<string, boolean>,
}));

vi.mock("@/store/global", () => ({ getApi: () => hub.api }));
vi.mock("@/util/logger", () => ({ Logger: { warn: vi.fn(), info: vi.fn(), error: vi.fn(), debug: vi.fn() } }));
vi.mock("@/util/host-caps", () => ({ hostHas: (cap: string) => hub.caps[cap] ?? false }));
vi.mock("@/app/drag/live-follow", () => ({
    beginLiveFollow: async (_api: unknown, spec: unknown) => void hub.begun.push(spec),
}));
vi.mock("@/app/drag/tearoff-snapshot", () => ({ peekWindowTabSnapshot: async () => hub.picture }));
vi.mock("./tab-grab-offset", () => ({ tabAnchorOffset: () => hub.offset }));
vi.mock("./tabbar-dnd", () => ({ TEAR_PAST_PX: 5 }));

import { setPlatform } from "@/util/platformutil";
import { armWindowTabDrag } from "./tab-live-follow";

let tab: HTMLElement;

function drag(overrides: Partial<Parameters<typeof armWindowTabDrag>[0]> = {}) {
    return armWindowTabDrag({
        tabId: "t1",
        workspaceId: "ws-1",
        windowLabel: "main",
        isLastTab: false,
        tabEl: tab,
        ...overrides,
    });
}

describe("arming a window-tab drag", () => {
    beforeEach(() => {
        setPlatform("win32");
        hub.picture = "jpeg";
        hub.offset = { x: 130, y: 25 };
        hub.begun = [];
        hub.caps = { tearOff: true };
        hub.api.startTabDragTracking.mockReset().mockResolvedValue(undefined);
        document.body.innerHTML = '<div class="tab-bar-scroll"><div class="tab-drop-wrapper" id="tab"></div></div>';
        tab = document.getElementById("tab") as HTMLElement;
        const strip = document.querySelector(".tab-bar-scroll") as HTMLElement;
        strip.getBoundingClientRect = () => ({ bottom: 36 }) as DOMRect;
    });
    afterEach(() => setPlatform("darwin"));

    it("arms the hook, then shows the window's picture below the strip", async () => {
        await drag();
        expect(hub.api.startTabDragTracking).toHaveBeenCalledWith({
            sourceWindowLabel: "main",
            tabId: "t1",
            sourceWsId: "ws-1",
            isLastTab: false,
        });
        expect(hub.begun).toEqual([
            {
                sourceWindowLabel: "main",
                picture: "jpeg",
                windowWidth: window.outerWidth,
                windowHeight: window.outerHeight,
                offsetX: 130,
                offsetY: 25,
                tearLineY: 41,
            },
        ]);
    });

    it("a lone tab never tears off, so there is no picture", async () => {
        await drag({ isLastTab: true });
        expect(hub.api.startTabDragTracking).toHaveBeenCalledTimes(1);
        expect(hub.begun).toEqual([]);
    });

    it("no picture, no grab offset or a host that cannot tear off: the drag is as it was", async () => {
        hub.picture = undefined;
        await drag();
        hub.picture = "jpeg";
        hub.offset = null;
        await drag();
        hub.offset = { x: 1, y: 1 };
        hub.caps = { tearOff: false };
        await drag();
        expect(hub.begun).toEqual([]);
    });

    it("off Windows only the hook is armed", async () => {
        setPlatform("linux");
        await drag();
        expect(hub.api.startTabDragTracking).toHaveBeenCalledTimes(1);
        expect(hub.begun).toEqual([]);
    });

    it("a hook that fails to start shows no picture", async () => {
        hub.api.startTabDragTracking.mockRejectedValue(new Error("hook"));
        await drag();
        expect(hub.begun).toEqual([]);
    });
});
