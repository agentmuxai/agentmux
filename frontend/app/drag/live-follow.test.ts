// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/util/logger", () => ({ Logger: { warn: vi.fn(), info: vi.fn(), error: vi.fn(), debug: vi.fn() } }));

import { setPlatform } from "@/util/platformutil";
import { beginLiveFollow, endLiveFollow, resizeLiveFollow, type LiveFollowSpec } from "./live-follow";

const spec: LiveFollowSpec = {
    sourceWindowLabel: "main",
    picture: "jpeg",
    windowWidth: 1200,
    windowHeight: 800,
    offsetX: 90,
    offsetY: 20,
    tearLineY: 42,
};

function fakeApi() {
    return { liveFollowBegin: vi.fn(async () => {}), liveFollowEnd: vi.fn(async () => {}), liveFollowResize: vi.fn(async () => {}) };
}

describe("live follow", () => {
    beforeEach(() => setPlatform("win32"));
    afterEach(() => setPlatform("darwin"));

    it("hands the host the picture and where it goes", async () => {
        const api = fakeApi();
        await beginLiveFollow(api, spec);
        expect(api.liveFollowBegin).toHaveBeenCalledWith(spec);
    });

    it("a refused picture is only lost: the failure is swallowed", async () => {
        const api = fakeApi();
        api.liveFollowBegin.mockRejectedValue(new Error("no window"));
        await expect(beginLiveFollow(api, spec)).resolves.toBeUndefined();
    });

    it("drops the picture when the real window is up", () => {
        const api = fakeApi();
        endLiveFollow(api);
        expect(api.liveFollowEnd).toHaveBeenCalledTimes(1);
    });

    it("gives the picture another size, rounded to whole DIP, with the calls kept in order", async () => {
        const api = fakeApi();
        const order: number[] = [];
        let release: () => void = () => {};
        api.liveFollowResize
            .mockImplementationOnce(() => new Promise<void>((resolve) => { release = () => { order.push(1); resolve(); }; }))
            .mockImplementationOnce(async () => void order.push(2));
        const first = resizeLiveFollow(api, { width: 600.4, height: 399.6 });
        const second = resizeLiveFollow(api, { width: 300, height: 200 });
        await Promise.resolve();
        expect(api.liveFollowResize).toHaveBeenCalledTimes(1);
        expect(api.liveFollowResize).toHaveBeenCalledWith({ windowWidth: 600, windowHeight: 400 });
        release();
        await Promise.all([first, second]);
        expect(order).toEqual([1, 2]);
        expect(api.liveFollowResize).toHaveBeenLastCalledWith({ windowWidth: 300, windowHeight: 200 });
    });

    it("a refused size is only lost: the next one still goes out", async () => {
        const api = fakeApi();
        api.liveFollowResize.mockRejectedValueOnce(new Error("no picture"));
        await resizeLiveFollow(api, { width: 10, height: 10 });
        await resizeLiveFollow(api, { width: 20, height: 20 });
        expect(api.liveFollowResize).toHaveBeenCalledTimes(2);
    });

    it("does nothing off Windows", async () => {
        setPlatform("linux");
        const api = fakeApi();
        await beginLiveFollow(api, spec);
        endLiveFollow(api);
        await resizeLiveFollow(api, { width: 1, height: 1 });
        expect(api.liveFollowBegin).not.toHaveBeenCalled();
        expect(api.liveFollowEnd).not.toHaveBeenCalled();
        expect(api.liveFollowResize).not.toHaveBeenCalled();
    });
});
